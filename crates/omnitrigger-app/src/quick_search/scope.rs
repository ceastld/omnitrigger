#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    Triggers,
    Wheels,
    TextHistory,
    Actions,
    Date,
    Timestamp,
}

pub(super) fn scoped_query(query: &str) -> Option<(Scope, &str)> {
    let (prefix, term) = query.split_once([':', '：'])?;
    let scope = match prefix.trim().to_ascii_lowercase().as_str() {
        "触发器" | "trigger" | "triggers" => Scope::Triggers,
        "轮盘" | "轨道轮盘" | "wheel" | "wheels" => Scope::Wheels,
        "剪贴板" | "文本历史" | "clipboard" => Scope::TextHistory,
        "动作" | "动作库" | "action" | "actions" => Scope::Actions,
        "日期" | "date" => Scope::Date,
        "时间戳" | "unix" | "timestamp" => Scope::Timestamp,
        _ => return None,
    };
    Some((scope, term.trim()))
}
