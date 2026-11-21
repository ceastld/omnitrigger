#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    Triggers,
    Wheels,
    WheelSlots,
    Presets,
    Settings,
    TextHistory,
    Actions,
    Date,
    Duration,
    Timestamp,
    Color,
}

pub(super) fn scoped_query(query: &str) -> Option<(Scope, &str)> {
    let (prefix, term) = query.split_once([':', '：'])?;
    let scope = match prefix.trim().to_ascii_lowercase().as_str() {
        "触发器" | "trigger" | "triggers" => Scope::Triggers,
        "轮盘" | "轨道轮盘" | "wheel" | "wheels" => Scope::Wheels,
        "槽位" | "轮盘槽位" | "slot" | "wheel-slot" => Scope::WheelSlots,
        "预设" | "触发预设" | "preset" | "presets" => Scope::Presets,
        "设置" | "setting" | "settings" => Scope::Settings,
        "剪贴板" | "文本历史" | "clipboard" => Scope::TextHistory,
        "动作" | "动作库" | "action" | "actions" => Scope::Actions,
        "日期" | "date" => Scope::Date,
        "时长" | "duration" => Scope::Duration,
        "时间戳" | "unix" | "timestamp" => Scope::Timestamp,
        "颜色" | "color" | "colour" => Scope::Color,
        _ => return None,
    };
    Some((scope, term.trim()))
}
