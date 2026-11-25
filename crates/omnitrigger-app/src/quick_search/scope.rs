use crate::trigger_browse::{FacetValue, ScopeKind, StatusKind};

use super::{
    calculator::narrow_ascii_graphic, json_tools::is_json_validation_operation,
    text_tools::preserves_operand_trailing_whitespace,
};

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
    BatchCalculation,
    ConversionHistory,
    QueryHistory,
    BatchConversion,
    Radix,
    Actions,
    Applications,
    ApplicationHistory,
    Folders,
    WindowsSettings,
    LocalFiles,
    OpenWindows,
    Date,
    Duration,
    VideoFrames,
    Timecode,
    TransferTime,
    ExactData,
    Pace,
    FuelEconomy,
    UnitPrice,
    Renminbi,
    Circuit,
    Period,
    Wave,
    Pitch,
    Decibel,
    Solid,
    Tempo,
    AspectRatio,
    MapScale,
    PixelDensity,
    Paper,
    Rectangle,
    Cidr,
    FileMode,
    Timestamp,
    Uuid,
    Ulid,
    Color,
    ColorContrast,
    TextCount,
    TextTools,
    Json,
    Toml,
    Csv,
    DataPipeline,
    Fraction,
    Equation,
    LinearSystem,
    LinearFit,
    Statistics,
    Triangle,
    Circle,
    Polygon,
    Complex,
    Vector,
    Matrix,
    Percentage,
    RatioAllocation,
    Intervals,
    ToolIndex,
    SearchShortcuts,
    ExampleHistory,
    Bookmarks,
    UrlEncoding,
    Path,
}

pub(super) fn scoped_query(query: &str) -> Option<(Scope, &str)> {
    if let Some((prefix, term)) = query.split_once([':', '：']) {
        let scope = parse_scope(prefix)?;
        let term = term.trim_start();
        let keep_trailing = scope == Scope::Csv
            || (scope == Scope::Json
                && term
                    .split_whitespace()
                    .next()
                    .is_some_and(is_json_validation_operation))
            || (scope == Scope::TextTools
                && term
                    .split_whitespace()
                    .next()
                    .is_some_and(preserves_operand_trailing_whitespace));
        let term = if keep_trailing { term } else { term.trim_end() };
        return Some((scope, term));
    }
    match parse_scope(query)? {
        scope @ (Scope::Triggers(Some(_)) | Scope::Bookmarks) => Some((scope, "")),
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
        Scope::Triggers(_) => Some("触发器:"),
        Scope::CalculationHistory => Some("算式历史:"),
        Scope::BatchCalculation => Some("批量计算:"),
        Scope::ConversionHistory => Some("换算历史:"),
        Scope::QueryHistory => Some("查询历史:"),
        Scope::Bookmarks => Some("收藏:"),
        Scope::BatchConversion => Some("批量换算:"),
        Scope::Radix => Some("进制:"),
        Scope::Applications => Some("应用:"),
        Scope::ApplicationHistory => Some("应用历史:"),
        Scope::Folders => Some("文件夹:"),
        Scope::WindowsSettings => Some("系统设置:"),
        Scope::LocalFiles => Some("文件:"),
        Scope::OpenWindows => Some("窗口:"),
        Scope::Date => Some("日期:"),
        Scope::Duration => Some("时长:"),
        Scope::VideoFrames => Some("帧数:"),
        Scope::Timecode => Some("时间码:"),
        Scope::TransferTime => Some("下载:"),
        Scope::ExactData => Some("数据换算:"),
        Scope::Pace => Some("配速:"),
        Scope::FuelEconomy => Some("油耗:"),
        Scope::UnitPrice => Some("单价:"),
        Scope::Renminbi => Some("人民币:"),
        Scope::Circuit => Some("电路:"),
        Scope::Period => Some("周期:"),
        Scope::Wave => Some("波长:"),
        Scope::Pitch => Some("音高:"),
        Scope::Decibel => Some("分贝:"),
        Scope::Solid => Some("立体:"),
        Scope::Tempo => Some("节拍:"),
        Scope::AspectRatio => Some("宽高比:"),
        Scope::MapScale => Some("比例尺:"),
        Scope::PixelDensity => Some("像素密度:"),
        Scope::Paper => Some("纸张:"),
        Scope::Rectangle => Some("区域:"),
        Scope::Cidr => Some("网段:"),
        Scope::FileMode => Some("权限:"),
        Scope::Timestamp => Some("时间戳"),
        Scope::Uuid => Some("UUID:"),
        Scope::Ulid => Some("ULID:"),
        Scope::Color => Some("颜色:"),
        Scope::ColorContrast => Some("对比度:"),
        Scope::TextCount => Some("字数:"),
        Scope::TextTools => Some("文本:"),
        Scope::Json => Some("JSON:"),
        Scope::Toml => Some("TOML:"),
        Scope::Csv => Some("CSV:"),
        Scope::DataPipeline => Some("处理:"),
        Scope::Fraction => Some("分数:"),
        Scope::Equation => Some("方程:"),
        Scope::LinearSystem => Some("联立:"),
        Scope::LinearFit => Some("拟合:"),
        Scope::Statistics => Some("统计:"),
        Scope::Triangle => Some("三角形:"),
        Scope::Circle => Some("圆:"),
        Scope::Polygon => Some("多边形:"),
        Scope::Complex => Some("复数:"),
        Scope::Vector => Some("向量:"),
        Scope::Matrix => Some("矩阵:"),
        Scope::Percentage => Some("百分比:"),
        Scope::RatioAllocation => Some("配比:"),
        Scope::Intervals => Some("区间:"),
        Scope::SearchShortcuts => Some("搜索快捷键:"),
        Scope::UrlEncoding => Some("URL:"),
        Scope::Path => Some("路径:"),
        _ => None,
    }
}

pub(super) fn parse_scope(prefix: &str) -> Option<Scope> {
    let prefix: String = prefix
        .trim()
        .chars()
        .map(|character| match character {
            '\u{3000}' => ' ',
            _ => narrow_ascii_graphic(character).to_ascii_lowercase(),
        })
        .collect();
    let scope = match prefix.as_str() {
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
        "批量计算" | "batch calculation" => Scope::BatchCalculation,
        "换算历史" | "conversion history" | "unit history" => Scope::ConversionHistory,
        "查询历史" | "query history" => Scope::QueryHistory,
        "批量换算" | "batch conversion" => Scope::BatchConversion,
        "进制" | "radix" | "base" => Scope::Radix,
        "动作" | "动作库" | "action" | "actions" => Scope::Actions,
        "应用" | "程序" | "app" | "apps" => Scope::Applications,
        "应用历史" | "app history" | "recent apps" => Scope::ApplicationHistory,
        "文件夹" | "目录" | "folder" | "folders" => Scope::Folders,
        "系统设置" | "windows settings" | "system settings" => Scope::WindowsSettings,
        "文件" | "文件名" | "file" | "files" => Scope::LocalFiles,
        "窗口" | "window" | "windows" => Scope::OpenWindows,
        "日期" | "date" => Scope::Date,
        "时长" | "duration" => Scope::Duration,
        "帧数" | "视频帧" | "frames" | "video frames" => Scope::VideoFrames,
        "时间码" | "timecode" => Scope::Timecode,
        "下载" | "下载时长" | "传输时长" | "download" => Scope::TransferTime,
        "数据换算" | "精确数据换算" | "data conversion" => Scope::ExactData,
        "配速" | "pace" => Scope::Pace,
        "油耗" | "百公里油耗" | "fuel economy" => Scope::FuelEconomy,
        "单价" | "单位价格" | "unit price" => Scope::UnitPrice,
        "人民币" | "大写金额" | "rmb" => Scope::Renminbi,
        "电路" | "欧姆定律" | "circuit" | "ohm's law" => Scope::Circuit,
        "周期" | "频率周期" | "period" => Scope::Period,
        "波长" | "频率波长" | "wave" | "wavelength" => Scope::Wave,
        "音高" | "音符" | "pitch" => Scope::Pitch,
        "分贝" | "db" | "decibel" => Scope::Decibel,
        "立体" | "solid" => Scope::Solid,
        "节拍" | "tempo" | "bpm" => Scope::Tempo,
        "宽高比" | "aspect" | "aspect ratio" => Scope::AspectRatio,
        "比例尺" | "map scale" => Scope::MapScale,
        "像素密度" | "ppi" | "pixel density" => Scope::PixelDensity,
        "纸张" | "纸张尺寸" | "paper" | "paper size" => Scope::Paper,
        "区域" | "矩形" | "屏幕区域" | "rectangle" => Scope::Rectangle,
        "网段" | "子网" | "cidr" | "subnet" => Scope::Cidr,
        "权限" | "文件权限" | "mode" | "permissions" => Scope::FileMode,
        "时间戳" | "unix" | "timestamp" => Scope::Timestamp,
        "uuid" => Scope::Uuid,
        "ulid" => Scope::Ulid,
        "颜色" | "color" | "colour" => Scope::Color,
        "对比度" | "颜色对比" | "contrast" => Scope::ColorContrast,
        "字数" | "text count" | "text length" => Scope::TextCount,
        "文本" | "文本工具" | "text tools" => Scope::TextTools,
        "json" => Scope::Json,
        "toml" => Scope::Toml,
        "csv" => Scope::Csv,
        "处理" | "数据处理" | "data pipeline" => Scope::DataPipeline,
        "分数" | "fraction" => Scope::Fraction,
        "方程" | "二次方程" | "equation" | "quadratic" => Scope::Equation,
        "联立" | "方程组" | "线性方程组" | "linear system" => Scope::LinearSystem,
        "拟合" | "线性拟合" | "回归" | "linear fit" => Scope::LinearFit,
        "统计" | "stats" | "statistics" => Scope::Statistics,
        "三角形" | "triangle" => Scope::Triangle,
        "圆" | "circle" => Scope::Circle,
        "多边形" | "polygon" => Scope::Polygon,
        "复数" | "complex" => Scope::Complex,
        "向量" | "vector" => Scope::Vector,
        "矩阵" | "matrix" => Scope::Matrix,
        "百分比" | "比例" | "percentage" => Scope::Percentage,
        "配比" | "按比例分配" | "ratio allocation" => Scope::RatioAllocation,
        "区间" | "intervals" | "interval" => Scope::Intervals,
        "工具" | "工具目录" | "tools" => Scope::ToolIndex,
        "搜索快捷键" | "search shortcuts" => Scope::SearchShortcuts,
        "用法历史" | "example history" => Scope::ExampleHistory,
        "收藏" | "bookmarks" => Scope::Bookmarks,
        "网址" | "url" => Scope::UrlEncoding,
        "路径" | "path" => Scope::Path,
        _ => return None,
    };
    Some(scope)
}
