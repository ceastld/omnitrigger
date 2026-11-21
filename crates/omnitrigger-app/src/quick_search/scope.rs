#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    Triggers,
    TextHistory,
    Actions,
}

pub(super) fn scoped_query(query: &str) -> Option<(Scope, &str)> {
    let (prefix, term) = query.split_once([':', '：'])?;
    let scope = match prefix.trim().to_ascii_lowercase().as_str() {
        "触发器" | "trigger" | "triggers" => Scope::Triggers,
        "剪贴板" | "文本历史" | "clipboard" => Scope::TextHistory,
        "动作" | "动作库" | "action" | "actions" => Scope::Actions,
        _ => return None,
    };
    Some((scope, term.trim()))
}
