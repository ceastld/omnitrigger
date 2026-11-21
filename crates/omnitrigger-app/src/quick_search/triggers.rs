use crate::{
    assets::Icon, binding_context_label, browse_row, caps_for, settings::Binding, trigger_browse,
};

#[derive(Clone)]
pub(super) struct SearchTrigger {
    row: trigger_browse::Row,
    title_key: String,
    pub(super) title: String,
    pub(super) detail: String,
    pub(super) icon: Icon,
}

impl SearchTrigger {
    pub(super) fn from_binding(binding: &Binding) -> Self {
        let row = browse_row(binding, true);
        let title = if binding.message.trim().is_empty() {
            row.action.label().into()
        } else {
            binding.message.clone()
        };
        let mut detail = caps_for(&binding.spec)
            .into_iter()
            .map(|cap| cap.label.to_string())
            .collect::<Vec<_>>()
            .join(" + ");
        if !binding.enabled {
            detail.push_str(" · 已停用");
        }
        if let Some(context) = binding_context_label(binding) {
            detail.push_str(" · ");
            detail.push_str(&context);
        }
        let (icon, _) = crate::action_look(&binding.action);
        Self {
            row,
            title_key: title.to_lowercase(),
            title,
            detail,
            icon,
        }
    }

    pub(super) fn id(&self) -> u64 {
        self.row.id
    }
}

pub(super) fn matching_triggers(
    triggers: &[SearchTrigger],
    query: &str,
    filter: Option<trigger_browse::FacetValue>,
) -> Vec<u64> {
    let query = trigger_browse::normalize_query(query);
    let mut matches: Vec<_> = triggers
        .iter()
        .enumerate()
        .filter(|(_, entry)| filter.is_none_or(|filter| filter.matches(&entry.row)))
        .filter(|(_, entry)| trigger_browse::matches_query(&entry.row, &query))
        .map(|(index, entry)| {
            let rank = super::title_rank(&entry.title_key, &query);
            (
                rank,
                entry.row.status == trigger_browse::StatusKind::Disabled,
                index,
            )
        })
        .collect();
    matches.sort_unstable();
    matches
        .into_iter()
        .map(|(_, _, index)| triggers[index].id())
        .collect()
}
