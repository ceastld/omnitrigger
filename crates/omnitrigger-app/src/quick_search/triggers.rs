use crate::{
    assets::Icon,
    binding_context_label, browse_row, caps_for,
    settings::Binding,
    trigger_browse::{self, Facet, FacetValue, Filter, StatusKind},
};

use super::examples::search_key;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TriggerFilterError {
    InvalidValue(Facet),
    RepeatedFacet,
}

impl TriggerFilterError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::InvalidValue(Facet::Status) => "状态可用 已启用 或 已停用",
            Self::InvalidValue(Facet::Scope) => "范围可用 所有应用 或 指定应用",
            Self::InvalidValue(Facet::Input) => "输入可用 键盘、鼠标按键、滚轮或屏幕区域",
            Self::InvalidValue(Facet::Action) => "动作填写触发器页的分类名称，例如 快速搜索",
            Self::RepeatedFacet => "每类筛选条件填写一次",
        }
    }
}

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

pub(super) fn trigger_search_filters(
    mut query: &str,
    inherited: Option<FacetValue>,
) -> Result<(Filter, &str), TriggerFilterError> {
    let mut filter = Filter::default();
    if let Some(value) = inherited {
        filter.set(value.facet(), Some(value));
    }
    while let Some((label, input)) = query.split_once(['=', '＝']) {
        let facet = match search_key(label).as_str() {
            "状态" | "status" | "state" => Facet::Status,
            "范围" | "scope" => Facet::Scope,
            "输入" | "input" => Facet::Input,
            "动作" | "action" => Facet::Action,
            _ => break,
        };
        if filter.get(facet).is_some() {
            return Err(TriggerFilterError::RepeatedFacet);
        }
        let (value, rest) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
        let value = search_key(value);
        let name = match (facet, value.as_str()) {
            (Facet::Status, "enabled") => "已启用",
            (Facet::Status, "disabled") => "已停用",
            (Facet::Scope, "全局" | "global") => "所有应用",
            (Facet::Scope, "app" | "scoped") => "指定应用",
            (Facet::Input, "keyboard") => "键盘",
            (Facet::Input, "mouse") => "鼠标按键",
            (Facet::Input, "wheel") => "滚轮",
            (Facet::Input, "region") => "屏幕区域",
            _ => value.as_str(),
        };
        let value = facet
            .options()
            .into_iter()
            .find(|value| {
                value.label() == name && *value != FacetValue::Status(StatusKind::Unready)
            })
            .ok_or(TriggerFilterError::InvalidValue(facet))?;
        filter.set(facet, Some(value));
        query = rest.trim_start();
    }
    Ok((filter, query))
}

pub(super) fn matching_triggers(
    triggers: &[SearchTrigger],
    query: &str,
    filter: &Filter,
) -> Vec<u64> {
    let query = trigger_browse::normalize_query(query);
    let unfiltered = filter.is_empty();
    let mut matches: Vec<_> = triggers
        .iter()
        .enumerate()
        .filter(|(_, entry)| unfiltered || filter.accepts(&entry.row, None))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trigger_browse::{ActionKind, InputKind, ScopeKind};

    #[test]
    fn parses_all_trigger_facets_and_keeps_the_chord_as_search_text() {
        let (filter, query) = trigger_search_filters(
            "状态＝已启用 SCOPE=GLOBAL 输入=键盘 动作=快速搜索 Space + S",
            None,
        )
        .unwrap();
        assert_eq!(filter.status, Some(StatusKind::Enabled));
        assert_eq!(filter.scope, Some(ScopeKind::Global));
        assert_eq!(filter.input, Some(InputKind::Keyboard));
        assert_eq!(filter.action, Some(ActionKind::Search));
        assert_eq!(query, "Space + S");
    }
}
