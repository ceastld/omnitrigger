#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    Triggers,
    Wheels,
    WheelSlots,
    Presets,
    Settings,
    TextHistory,
    TriggerHistory,
    CalculationHistory,
    Actions,
    Date,
    Duration,
    Timestamp,
    Color,
    TextCount,
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
        "触发记录" | "trigger history" | "hits" => Scope::TriggerHistory,
        "算式历史" | "calc history" => Scope::CalculationHistory,
        "动作" | "动作库" | "action" | "actions" => Scope::Actions,
        "日期" | "date" => Scope::Date,
        "时长" | "duration" => Scope::Duration,
        "时间戳" | "unix" | "timestamp" => Scope::Timestamp,
        "颜色" | "color" | "colour" => Scope::Color,
        "字数" | "text count" | "text length" => Scope::TextCount,
        _ => return None,
    };
    Some((scope, term.trim()))
}
