use crate::trigger_browse::{FacetValue, ScopeKind, StatusKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    Triggers(Option<FacetValue>),
    Wheels,
    WheelSlots,
    Presets,
    Settings,
    TextHistory,
    TriggerHistory,
    CalculationHistory,
    ConversionHistory,
    Actions,
    Applications,
    Date,
    Duration,
    TransferTime,
    AspectRatio,
    Cidr,
    Timestamp,
    Color,
    ColorContrast,
    TextCount,
    TextTools,
    Json,
    Fraction,
    Percentage,
    UrlEncoding,
}

pub(super) fn scoped_query(query: &str) -> Option<(Scope, &str)> {
    if let Some((prefix, term)) = query.split_once([':', '：']) {
        return Some((parse_scope(prefix)?, term.trim()));
    }
    match parse_scope(query)? {
        scope @ Scope::Triggers(Some(_)) => Some((scope, "")),
        _ => None,
    }
}

pub(super) fn tool_example_keyword(query: &str) -> Option<&'static str> {
    match query {
        "计算器" | "calculator" => return Some("计算"),
        "单位换算" | "换算" | "convert" | "units" => return Some("换算"),
        _ => {}
    }
    match parse_scope(query)? {
        Scope::CalculationHistory => Some("算式历史:"),
        Scope::ConversionHistory => Some("换算历史:"),
        Scope::Applications => Some("应用:"),
        Scope::Date => Some("日期:"),
        Scope::Duration => Some("时长:"),
        Scope::TransferTime => Some("下载:"),
        Scope::AspectRatio => Some("宽高比:"),
        Scope::Cidr => Some("网段:"),
        Scope::Timestamp => Some("时间戳"),
        Scope::Color => Some("颜色:"),
        Scope::ColorContrast => Some("对比度:"),
        Scope::TextCount => Some("字数:"),
        Scope::TextTools => Some("文本:"),
        Scope::Json => Some("JSON:"),
        Scope::Fraction => Some("分数:"),
        Scope::Percentage => Some("百分比:"),
        Scope::UrlEncoding => Some("URL:"),
        _ => None,
    }
}

fn parse_scope(prefix: &str) -> Option<Scope> {
    let scope = match prefix.trim().to_ascii_lowercase().as_str() {
        "触发器" | "trigger" | "triggers" => Scope::Triggers(None),
        "已启用触发器" | "enabled triggers" => {
            Scope::Triggers(Some(FacetValue::Status(StatusKind::Enabled)))
        }
        "已停用触发器" | "disabled triggers" => {
            Scope::Triggers(Some(FacetValue::Status(StatusKind::Disabled)))
        }
        "全局触发器" | "global triggers" => {
            Scope::Triggers(Some(FacetValue::Scope(ScopeKind::Global)))
        }
        "指定应用触发器" | "app triggers" => {
            Scope::Triggers(Some(FacetValue::Scope(ScopeKind::Scoped)))
        }
        "轮盘" | "轨道轮盘" | "wheel" | "wheels" => Scope::Wheels,
        "槽位" | "轮盘槽位" | "slot" | "wheel-slot" => Scope::WheelSlots,
        "预设" | "触发预设" | "preset" | "presets" => Scope::Presets,
        "设置" | "setting" | "settings" => Scope::Settings,
        "剪贴板" | "文本历史" | "clipboard" => Scope::TextHistory,
        "触发记录" | "trigger history" | "hits" => Scope::TriggerHistory,
        "算式历史" | "calc history" => Scope::CalculationHistory,
        "换算历史" | "conversion history" | "unit history" => Scope::ConversionHistory,
        "动作" | "动作库" | "action" | "actions" => Scope::Actions,
        "应用" | "程序" | "app" | "apps" => Scope::Applications,
        "日期" | "date" => Scope::Date,
        "时长" | "duration" => Scope::Duration,
        "下载" | "下载时长" | "传输时长" | "download" => Scope::TransferTime,
        "宽高比" | "aspect" | "aspect ratio" => Scope::AspectRatio,
        "网段" | "子网" | "cidr" | "subnet" => Scope::Cidr,
        "时间戳" | "unix" | "timestamp" => Scope::Timestamp,
        "颜色" | "color" | "colour" => Scope::Color,
        "对比度" | "颜色对比" | "contrast" => Scope::ColorContrast,
        "字数" | "text count" | "text length" => Scope::TextCount,
        "文本" | "文本工具" | "text tools" => Scope::TextTools,
        "json" => Scope::Json,
        "分数" | "fraction" => Scope::Fraction,
        "百分比" | "比例" | "percentage" => Scope::Percentage,
        "网址" | "url" => Scope::UrlEncoding,
        _ => return None,
    };
    Some(scope)
}
