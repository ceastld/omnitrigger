use std::collections::BTreeMap;

use regex::Regex;
use serde::Serialize;

use crate::{
    assets::Icon,
    binding_context_label, browse_row, caps_for,
    settings::Binding,
    trigger_browse::{self, Facet, FacetValue, Filter, InputKind, ScopeKind, StatusKind},
};

use super::{
    csv_tools::append_bounded_csv_row,
    examples::search_key,
    operators::query_option_value,
    text_tools::{compare_natural, compile_regex},
    Destination, InputSuggestion, Results, ValueError, INPUT_CHARACTER_LIMIT,
};

const TRIGGER_CSV_HEADER: &str = "名称,组合与条件,输入,动作,范围,启用状态";
const MAX_TRIGGER_JSON_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy)]
enum TriggerExport {
    Csv,
    Json,
    ActionCounts,
}

impl TriggerExport {
    const ALL: [Self; 3] = [Self::Csv, Self::Json, Self::ActionCounts];

    fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::Csv => &["CSV"],
            Self::Json => &["JSON"],
            Self::ActionCounts => &["动作统计", "action-counts"],
        }
    }
}

#[derive(Serialize)]
struct TriggerSummary<'a> {
    name: &'a str,
    detail: &'a str,
    input: &'static str,
    action: &'static str,
    scope: &'static str,
    enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TriggerSearchError {
    InvalidValue { facet: Facet, start: usize },
    RepeatedFacet,
    ExportTooLarge,
    JsonExport,
    InvalidExport,
    TitleRegex,
    SortOrder { start: usize },
    RepeatedSortOrder,
}

impl TriggerSearchError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::InvalidValue { facet, .. } => match facet {
                Facet::Status => "状态可用 已启用 或 已停用",
                Facet::Scope => "范围可用 所有应用 或 指定应用",
                Facet::Input => "输入可用 键盘、鼠标按键、滚轮或屏幕区域",
                Facet::Action => "动作填写触发器页的分类名称，例如 快速搜索",
            },
            Self::RepeatedFacet => "每类筛选条件填写一次",
            Self::ExportTooLarge => "导出的触发器超过 1 MiB，请缩小筛选范围",
            Self::JsonExport => "生成触发器 JSON 失败",
            Self::InvalidExport => "触发器输出可用 CSV、JSON 或动作统计",
            Self::TitleRegex => "填写一个有效的名称正则表达式，含空格时用双引号包住",
            Self::SortOrder { .. } => "触发器排序可用 名称 或 动作",
            Self::RepeatedSortOrder => "触发器排序条件填写一次",
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

struct TriggerQuery<'a> {
    filter: Filter,
    term: &'a str,
    title_regex: Option<Regex>,
    order: Option<TriggerOrder>,
}

#[derive(Clone, Copy)]
enum TriggerOrder {
    Name,
    Action,
}

impl TriggerOrder {
    const ALL: [Self; 2] = [Self::Name, Self::Action];

    fn names(self) -> [&'static str; 2] {
        match self {
            Self::Name => ["名称", "name"],
            Self::Action => ["动作", "action"],
        }
    }

    fn suggestions(query_prefix: &str, value_prefix: &str) -> Vec<InputSuggestion> {
        let prefix = search_key(value_prefix);
        Self::ALL
            .into_iter()
            .filter_map(|order| {
                let name = order
                    .names()
                    .into_iter()
                    .find(|name| name.starts_with(&prefix))?;
                option_suggestion(query_prefix, name)
            })
            .collect()
    }

    fn sort(self, matches: Vec<&SearchTrigger>) -> Vec<&SearchTrigger> {
        let mut entries: Vec<_> = matches
            .into_iter()
            .map(|trigger| (search_key(&trigger.title), trigger))
            .collect();
        entries.sort_by(|(left_name, left), (right_name, right)| {
            let by_name = || compare_natural(left_name, right_name);
            match self {
                Self::Name => by_name(),
                Self::Action => left
                    .row
                    .action
                    .label()
                    .cmp(right.row.action.label())
                    .then_with(by_name),
            }
        });
        entries.into_iter().map(|(_, trigger)| trigger).collect()
    }
}

fn facet_values(facet: Facet) -> impl Iterator<Item = FacetValue> {
    facet
        .options()
        .into_iter()
        .filter(|value| *value != FacetValue::Status(StatusKind::Unready))
}

fn facet_value_names(value: FacetValue) -> impl Iterator<Item = &'static str> {
    let aliases: &[&str] = match value {
        FacetValue::Status(StatusKind::Enabled) => &["enabled"],
        FacetValue::Status(StatusKind::Disabled) => &["disabled"],
        FacetValue::Scope(ScopeKind::Global) => &["全局", "global"],
        FacetValue::Scope(ScopeKind::Scoped) => &["app", "scoped"],
        FacetValue::Input(InputKind::Keyboard) => &["keyboard"],
        FacetValue::Input(InputKind::MouseButton) => &["mouse"],
        FacetValue::Input(InputKind::Wheel) => &["wheel"],
        FacetValue::Input(InputKind::PointerRegion) => &["region"],
        _ => &[],
    };
    std::iter::once(value.label()).chain(aliases.iter().copied())
}

fn filter_suggestions(
    facet: Facet,
    query_prefix: &str,
    value_prefix: &str,
) -> Vec<InputSuggestion> {
    let prefix = search_key(value_prefix);
    facet_values(facet)
        .filter_map(|value| {
            let name = facet_value_names(value).find(|name| name.starts_with(&prefix))?;
            option_suggestion(query_prefix, name)
        })
        .collect()
}

fn option_suggestion(query_prefix: &str, name: &'static str) -> Option<InputSuggestion> {
    let expression = format!("{query_prefix}{name}");
    (expression.chars().count() <= INPUT_CHARACTER_LIMIT).then(|| InputSuggestion {
        expression,
        label: Some(name),
        result: String::new(),
    })
}

fn parse_trigger_query(
    mut query: &str,
    inherited: Option<FacetValue>,
) -> Result<TriggerQuery<'_>, TriggerSearchError> {
    let query_len = query.len();
    let mut filter = Filter::default();
    let mut title_regex = None;
    let mut order = None;
    if let Some(value) = inherited {
        filter.set(value.facet(), Some(value));
    }
    while let Some((label, input)) = query.split_once(['=', '＝']) {
        let facet = match search_key(label).as_str() {
            "状态" | "status" | "state" => Facet::Status,
            "范围" | "scope" => Facet::Scope,
            "输入" | "input" => Facet::Input,
            "动作" | "action" => Facet::Action,
            "排序" | "sort" | "order" => {
                if order.is_some() {
                    return Err(TriggerSearchError::RepeatedSortOrder);
                }
                let (value, rest) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
                let value = search_key(value);
                order = Some(
                    TriggerOrder::ALL
                        .into_iter()
                        .find(|order| order.names().contains(&value.as_str()))
                        .ok_or(TriggerSearchError::SortOrder {
                            start: query_len - input.len(),
                        })?,
                );
                query = rest.trim_start();
                continue;
            }
            "名称正则" | "标题正则" | "name-regex" => {
                if title_regex.is_some() {
                    return Err(TriggerSearchError::TitleRegex);
                }
                let (pattern, rest) = query_option_value(input, |character| character == '"')
                    .ok_or(TriggerSearchError::TitleRegex)?;
                title_regex =
                    Some(compile_regex(pattern).map_err(|_| TriggerSearchError::TitleRegex)?);
                query = rest;
                continue;
            }
            _ => break,
        };
        if filter.get(facet).is_some() {
            return Err(TriggerSearchError::RepeatedFacet);
        }
        let (value, rest) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
        let value = search_key(value);
        let value = facet_values(facet)
            .find(|option| facet_value_names(*option).any(|name| name == value))
            .ok_or(TriggerSearchError::InvalidValue {
                facet,
                start: query_len - input.len(),
            })?;
        filter.set(facet, Some(value));
        query = rest.trim_start();
    }
    Ok(TriggerQuery {
        filter,
        term: query,
        title_regex,
        order,
    })
}

pub(super) fn search_triggers(
    triggers: &[SearchTrigger],
    full_query: &str,
    term: &str,
    inherited: Option<FacetValue>,
) -> Results {
    let (first, rest) = term.split_once(char::is_whitespace).unwrap_or((term, ""));
    let (export, query) = match first.split_once(['=', '＝']) {
        Some((label, value)) if matches!(search_key(label).as_str(), "输出" | "output") => {
            let key = search_key(value);
            let export = TriggerExport::ALL.into_iter().find(|format| {
                format
                    .keywords()
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(&key))
            });
            if export.is_none() {
                if rest.is_empty() {
                    let start = full_query.len() - term.len() + first.len() - value.len();
                    let suggestions: Vec<_> = TriggerExport::ALL
                        .into_iter()
                        .filter_map(|format| {
                            format
                                .keywords()
                                .iter()
                                .copied()
                                .find(|name| name.to_ascii_lowercase().starts_with(&key))
                        })
                        .filter_map(|name| option_suggestion(&full_query[..start], name))
                        .collect();
                    if !suggestions.is_empty() {
                        return Results::FilterOptions(suggestions);
                    }
                }
                return Results::Value(Err(ValueError::TriggerSearch(
                    TriggerSearchError::InvalidExport,
                )));
            }
            (export, rest.trim_start())
        }
        _ => (None, term),
    };
    let query = match parse_trigger_query(query, inherited) {
        Ok(parsed) => parsed,
        Err(error) => {
            let parts = |start| full_query.split_at(full_query.len() - query.len() + start);
            let suggestions = match error {
                TriggerSearchError::InvalidValue { facet, start } => {
                    let (prefix, value) = parts(start);
                    filter_suggestions(facet, prefix, value)
                }
                TriggerSearchError::SortOrder { start } => {
                    let (prefix, value) = parts(start);
                    TriggerOrder::suggestions(prefix, value)
                }
                _ => Vec::new(),
            };
            return if suggestions.is_empty() {
                Results::Value(Err(ValueError::TriggerSearch(error)))
            } else {
                Results::FilterOptions(suggestions)
            };
        }
    };
    let mut matches = matching_triggers(
        triggers,
        query.term,
        &query.filter,
        query.title_regex.as_ref(),
    );
    if let Some(order) = query
        .order
        .filter(|_| !matches!(export, Some(TriggerExport::ActionCounts)))
    {
        matches = order.sort(matches);
    }
    match export {
        Some(TriggerExport::Csv) => {
            Results::text_value(export_trigger_csv(matches).map_err(ValueError::TriggerSearch))
        }
        Some(TriggerExport::Json) => {
            Results::text_value(export_trigger_json(matches).map_err(ValueError::TriggerSearch))
        }
        Some(TriggerExport::ActionCounts) => {
            Results::text_value(export_action_counts(matches).map_err(ValueError::TriggerSearch))
        }
        None => Results::Destinations(
            matches
                .into_iter()
                .map(|trigger| Destination::Trigger(trigger.id()))
                .collect(),
        ),
    }
}

fn export_action_counts(triggers: Vec<&SearchTrigger>) -> Result<String, TriggerSearchError> {
    let mut counts = BTreeMap::new();
    for trigger in triggers {
        *counts.entry(trigger.row.action.label()).or_insert(0usize) += 1;
    }
    let mut counts: Vec<_> = counts.into_iter().collect();
    counts.sort_by(|(left, left_count), (right, right_count)| {
        right_count.cmp(left_count).then_with(|| left.cmp(right))
    });
    let mut csv = String::from("动作,数量");
    for (action, count) in counts {
        let count = count.to_string();
        append_bounded_csv_row(
            &mut csv,
            [action, count.as_str()].into_iter(),
            TriggerSearchError::ExportTooLarge,
        )?;
    }
    Ok(csv)
}

fn export_trigger_csv(triggers: Vec<&SearchTrigger>) -> Result<String, TriggerSearchError> {
    let mut csv = String::from(TRIGGER_CSV_HEADER);
    for entry in triggers {
        append_bounded_csv_row(
            &mut csv,
            [
                entry.title.as_str(),
                entry.detail.as_str(),
                entry.row.input.label(),
                entry.row.action.label(),
                entry.row.scope.label(),
                entry.row.status.label(),
            ]
            .into_iter(),
            TriggerSearchError::ExportTooLarge,
        )?;
    }
    Ok(csv)
}

fn export_trigger_json(triggers: Vec<&SearchTrigger>) -> Result<String, TriggerSearchError> {
    let summaries: Vec<_> = triggers
        .into_iter()
        .map(|entry| TriggerSummary {
            name: &entry.title,
            detail: &entry.detail,
            input: entry.row.input.label(),
            action: entry.row.action.label(),
            scope: entry.row.scope.label(),
            enabled: entry.row.status == StatusKind::Enabled,
        })
        .collect();
    let json =
        serde_json::to_string_pretty(&summaries).map_err(|_| TriggerSearchError::JsonExport)?;
    if json.len() > MAX_TRIGGER_JSON_BYTES {
        return Err(TriggerSearchError::ExportTooLarge);
    }
    Ok(json)
}

pub(super) fn matching_triggers<'a>(
    triggers: &'a [SearchTrigger],
    query: &str,
    filter: &Filter,
    title_regex: Option<&Regex>,
) -> Vec<&'a SearchTrigger> {
    let query = trigger_browse::normalize_query(query);
    let unfiltered = filter.is_empty();
    let mut matches: Vec<_> = triggers
        .iter()
        .enumerate()
        .filter(|(_, entry)| unfiltered || filter.accepts(&entry.row, None))
        .filter(|(_, entry)| title_regex.is_none_or(|pattern| pattern.is_match(&entry.title)))
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
        .map(|(_, _, index)| &triggers[index])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trigger_browse::ActionKind;

    #[test]
    fn parses_all_trigger_facets_and_keeps_the_chord_as_search_text() {
        let parsed = parse_trigger_query(
            "状态＝已启用 SCOPE=GLOBAL 输入=键盘 动作=快速搜索 Space + S",
            None,
        )
        .unwrap();
        let filter = parsed.filter;
        assert_eq!(filter.status, Some(StatusKind::Enabled));
        assert_eq!(filter.scope, Some(ScopeKind::Global));
        assert_eq!(filter.input, Some(InputKind::Keyboard));
        assert_eq!(filter.action, Some(ActionKind::Search));
        assert_eq!(parsed.term, "Space + S");
    }

    #[test]
    fn preserves_the_original_trigger_name_pattern_and_combines_it_with_filters() {
        let parsed =
            parse_trigger_query(r#"名称正则＝"^Open Ａ\d+$" 状态=已启用 Caps + Space"#, None)
                .unwrap();
        assert_eq!(parsed.filter.status, Some(StatusKind::Enabled));
        assert_eq!(parsed.term, "Caps + Space");
        let pattern = parsed.title_regex.unwrap();
        assert!(pattern.is_match("Open Ａ12"));
        assert!(!pattern.is_match("open Ａ12"));
        assert!(!pattern.is_match("Open A12"));
        for query in ["名称正则=", "名称正则=[", "名称正则=a 名称正则=b"] {
            assert!(matches!(
                parse_trigger_query(query, None),
                Err(TriggerSearchError::TitleRegex)
            ));
        }
    }
}
