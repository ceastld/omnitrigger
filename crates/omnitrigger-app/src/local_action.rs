//! 内置动作的保存契约。步骤有序执行，模板只引用本次动作已定义的变量。
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_STEPS: usize = 32;
pub const MAX_TEXT_UNITS: usize = 65_536;
const MAX_CONFIG_BYTES: usize = 256 * 1024;
const MAX_VARIABLES: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalAction {
    pub version: u32,
    pub steps: Vec<LocalStep>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LocalStep {
    SendShortcut {
        keys: Vec<u16>,
    },
    KeyTap {
        key: u16,
    },
    KeySequence {
        modifiers: Vec<u16>,
        keys: Vec<u16>,
    },
    InputText {
        text: String,
    },
    SetClipboard {
        text: String,
    },
    ReadClipboard {
        variable: String,
    },
    SetVariable {
        name: String,
        value: String,
    },
    Open {
        target: String,
    },
    Delay {
        milliseconds: u32,
    },
    /// 向原窗口发送 Ctrl+C 并读取复制出的文本；结果保留在剪贴板。
    CopySelection {
        variable: String,
    },
    /// 读取原窗口标题；`strip_app_name` 去掉末尾的“ - 应用名”。
    ReadWindowTitle {
        variable: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        strip_app_name: bool,
    },
    /// 打开文本中找到的网址，最多 MAX_OPEN_URLS 个；没有网址时失败。
    OpenUrls {
        text: String,
    },
    /// 将查询文本编码后替换网址模板中的 %s 并打开。
    WebSearch {
        url: String,
        query: String,
    },
}

pub const MAX_OPEN_URLS: usize = 8;
const MAX_QUERY_CHARS: usize = 128;

impl LocalAction {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err(format!(
                "不支持本地动作版本 {}，请保留原配置。",
                self.version
            ));
        }
        if self.steps.is_empty() || self.steps.len() > MAX_STEPS {
            return Err(format!("本地动作需要 1–{MAX_STEPS} 个步骤。"));
        }
        // Some 是能在保存时确定的值；None 是运行时才读取的剪贴板值。
        let mut variables: BTreeMap<String, Option<String>> = BTreeMap::new();
        let mut delay = 0u32;
        for (index, step) in self.steps.iter().enumerate() {
            let result = (|| {
                match step {
                    LocalStep::SendShortcut { keys } => {
                        omnitrigger_windows::validate_shortcut(keys)?;
                    }
                    LocalStep::KeyTap { key } => {
                        omnitrigger_windows::validate_key_tap(*key)?;
                    }
                    LocalStep::KeySequence { modifiers, keys } => {
                        omnitrigger_windows::validate_key_sequence(modifiers, keys)?;
                    }
                    LocalStep::InputText { text } | LocalStep::SetClipboard { text } => {
                        validate_template(text, &variables)?;
                    }
                    LocalStep::ReadClipboard { variable }
                    | LocalStep::CopySelection { variable }
                    | LocalStep::ReadWindowTitle { variable, .. } => {
                        define_variable(variable, None, &mut variables)?;
                    }
                    LocalStep::OpenUrls { text } => {
                        if let Some(resolved) = validate_template(text, &variables)? {
                            if extract_urls(&resolved).is_empty() {
                                return Err("文本中没有网址。".into());
                            }
                        }
                    }
                    LocalStep::WebSearch { url, query } => {
                        search_url(url, "x")?;
                        validate_template(query, &variables)?;
                    }
                    LocalStep::SetVariable { name, value } => {
                        let resolved = validate_template(value, &variables)?;
                        define_variable(name, resolved, &mut variables)?;
                    }
                    LocalStep::Open { target } => {
                        if let Some(resolved) = validate_template(target, &variables)? {
                            omnitrigger_windows::validate_open_target(&resolved)?;
                        }
                    }
                    LocalStep::Delay { milliseconds } => {
                        if *milliseconds > 5_000 {
                            return Err("单次等待不能超过 5000 毫秒。".into());
                        }
                        delay += milliseconds;
                        if delay > 10_000 {
                            return Err("动作的显式等待总和不能超过 10000 毫秒。".into());
                        }
                    }
                }
                Ok(())
            })();
            result.map_err(|error: String| format!("第 {} 步：{error}", index + 1))?;
        }
        let bytes = serde_json::to_vec(self).map_err(|error| format!("动作无法序列化：{error}"))?;
        if bytes.len() > MAX_CONFIG_BYTES {
            return Err("本地动作配置不能超过 256 KiB。".into());
        }
        Ok(())
    }

    pub fn summary(&self) -> String {
        format!("本地动作 · {} 步", self.steps.len())
    }
}

impl LocalStep {
    /// 用于默认运行日志；不包含文本、剪贴板正文或展开后的目标。
    pub fn description(&self) -> String {
        match self {
            Self::SendShortcut { keys } => {
                format!(
                    "发送快捷键 {}",
                    keys.iter()
                        .map(|key| omnitrigger_windows::format_button(
                            &omnitrigger_core::Button::Key(*key)
                        ))
                        .collect::<Vec<_>>()
                        .join(" + ")
                )
            }
            Self::KeyTap { key } => format!(
                "点按 {}",
                omnitrigger_windows::format_button(&omnitrigger_core::Button::Key(*key))
            ),
            Self::KeySequence { keys, .. } => format!("发送按键序列（{} 键）", keys.len()),
            Self::InputText { text } => {
                format!("输入文本（{} 字符单元）", text.encode_utf16().count())
            }
            Self::SetClipboard { text } => {
                format!("写入剪贴板（{} 字符单元）", text.encode_utf16().count())
            }
            Self::ReadClipboard { variable } => format!("读取剪贴板到变量 {variable}"),
            Self::SetVariable { name, value } => format!(
                "设置变量 {name}（{} 字符单元）",
                value.encode_utf16().count()
            ),
            Self::Open { target } => {
                format!("打开目标（{} 字符单元）", target.encode_utf16().count())
            }
            Self::Delay { milliseconds } => format!("等待 {milliseconds} 毫秒"),
            Self::CopySelection { variable } => format!("复制选中文本到变量 {variable}"),
            Self::ReadWindowTitle { variable, .. } => format!("读取窗口标题到变量 {variable}"),
            Self::OpenUrls { text } => {
                format!(
                    "打开文本中的网址（{} 字符单元）",
                    text.encode_utf16().count()
                )
            }
            Self::WebSearch { .. } => "网页搜索".into(),
        }
    }
}

/// 按出现顺序提取去重的 http(s) 与 www. 网址，去掉句末标点，最多 MAX_OPEN_URLS 个。
pub(crate) fn extract_urls(text: &str) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    let separator = |c: char| c.is_whitespace() || "<>\"'`，。；、（）【】「」《》".contains(c);
    for token in text.split(separator) {
        let lower = token.to_ascii_lowercase();
        let Some(start) = ["https://", "http://", "www."]
            .iter()
            .filter_map(|prefix| lower.find(prefix))
            .min()
        else {
            continue;
        };
        let candidate = token[start..].trim_end_matches(|c: char| ".,;:!?)]}".contains(c));
        let url = if candidate.len() >= 4 && candidate[..4].eq_ignore_ascii_case("www.") {
            format!("https://{candidate}")
        } else {
            candidate.to_string()
        };
        if omnitrigger_windows::validate_open_target(&url).is_ok() && !urls.contains(&url) {
            urls.push(url);
            if urls.len() == MAX_OPEN_URLS {
                break;
            }
        }
    }
    urls
}

/// 将查询编码进只含一个 %s 的 http(s) 模板；查询先去首尾空白并截取前 128 个字符。
pub(crate) fn search_url(template: &str, query: &str) -> Result<String, String> {
    let lower = template.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://"))
        || template.matches("%s").count() != 1
        || template.chars().any(char::is_whitespace)
    {
        return Err("搜索网址需要以 http(s):// 开头、不含空白，并只包含一个 %s。".into());
    }
    let query: String = query.trim().chars().take(MAX_QUERY_CHARS).collect();
    if query.is_empty() {
        return Err("搜索内容为空。".into());
    }
    let mut encoded = String::new();
    for byte in query.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    let url = template.replacen("%s", &encoded, 1);
    omnitrigger_windows::validate_open_target(&url)?;
    Ok(url)
}

/// 去掉标题末尾的“ - 应用名”或“ — 应用名”；没有分隔符时原样返回。
pub(crate) fn strip_app_name(title: &str) -> &str {
    [" - ", " — "]
        .iter()
        .filter_map(|separator| title.rfind(separator))
        .max()
        .map(|index| title[..index].trim_end())
        .filter(|head| !head.is_empty())
        .unwrap_or(title)
}

fn validate_name(name: &str) -> Result<(), String> {
    let mut bytes = name.bytes();
    if name.is_empty()
        || name.len() > 32
        || !bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err("变量名需要 1–32 个 ASCII 字母、数字或下划线，且不能以数字开头。".into());
    }
    Ok(())
}

fn define_variable(
    name: &str,
    value: Option<String>,
    variables: &mut BTreeMap<String, Option<String>>,
) -> Result<(), String> {
    validate_name(name)?;
    if !variables.contains_key(name) && variables.len() == MAX_VARIABLES {
        return Err("单个动作不能定义超过 16 个变量。".into());
    }
    variables.insert(name.to_string(), value);
    Ok(())
}

pub(crate) fn validate_text(text: &str) -> Result<(), String> {
    if text.contains('\0') {
        return Err("文本不能包含空字符。".into());
    }
    if text.len() > MAX_TEXT_UNITS * 4
        || text.encode_utf16().take(MAX_TEXT_UNITS + 1).count() > MAX_TEXT_UNITS
    {
        return Err("文本不能超过 65536 个 UTF-16 字符单元。".into());
    }
    Ok(())
}

enum TemplatePart<'a> {
    Text(&'a str),
    Dollar,
    Variable(&'a str),
}

fn parse_template(text: &str) -> Result<Vec<TemplatePart<'_>>, String> {
    validate_text(text)?;
    let mut parts = Vec::new();
    let mut start = 0;
    let mut cursor = 0;
    let bytes = text.as_bytes();
    while cursor < bytes.len() {
        if bytes[cursor] != b'$' || !matches!(bytes.get(cursor + 1), Some(b'$' | b'{')) {
            cursor += 1;
            continue;
        }
        if start < cursor {
            parts.push(TemplatePart::Text(&text[start..cursor]));
        }
        if bytes[cursor + 1] == b'$' {
            parts.push(TemplatePart::Dollar);
            cursor += 2;
        } else {
            let name_start = cursor + 2;
            let Some(end) = text[name_start..].find('}').map(|end| name_start + end) else {
                return Err("模板变量缺少结束符 }。".into());
            };
            let name = &text[name_start..end];
            validate_name(name)?;
            parts.push(TemplatePart::Variable(name));
            cursor = end + 1;
        }
        start = cursor;
    }
    if start < text.len() {
        parts.push(TemplatePart::Text(&text[start..]));
    }
    Ok(parts)
}

fn validate_template(
    text: &str,
    variables: &BTreeMap<String, Option<String>>,
) -> Result<Option<String>, String> {
    let mut result = String::new();
    let mut units = 0;
    let mut dynamic = false;
    for part in parse_template(text)? {
        match part {
            TemplatePart::Text(value) => append_expansion(&mut result, &mut units, value)?,
            TemplatePart::Dollar => append_expansion(&mut result, &mut units, "$")?,
            TemplatePart::Variable(name) => match variables.get(name) {
                Some(Some(value)) => append_expansion(&mut result, &mut units, value)?,
                Some(None) => dynamic = true,
                None => {
                    return Err(format!(
                        "变量 {name} 尚未定义；只能引用前面步骤定义的变量。"
                    ))
                }
            },
        }
    }
    Ok((!dynamic).then_some(result))
}

/// 只展开明确提供的本次运行变量；不读取环境变量，也不递归解释变量值。
pub(crate) fn expand_template(
    text: &str,
    variables: &BTreeMap<String, String>,
) -> Result<String, String> {
    let mut result = String::new();
    let mut units = 0;
    for part in parse_template(text)? {
        let value = match part {
            TemplatePart::Text(value) => value,
            TemplatePart::Dollar => "$",
            TemplatePart::Variable(name) => variables
                .get(name)
                .ok_or_else(|| format!("变量 {name} 尚未定义。"))?,
        };
        append_expansion(&mut result, &mut units, value)?;
    }
    Ok(result)
}

fn append_expansion(result: &mut String, units: &mut usize, value: &str) -> Result<(), String> {
    validate_text(value)?;
    *units += value.encode_utf16().count();
    if *units > MAX_TEXT_UNITS {
        return Err("模板展开后的文本不能超过 65536 个 UTF-16 字符单元。".into());
    }
    result.push_str(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(steps: Vec<LocalStep>) -> LocalAction {
        LocalAction { version: 1, steps }
    }

    #[test]
    fn templates_are_ordered_explicit_and_not_recursive() {
        let local = action(vec![
            LocalStep::SetVariable {
                name: "title".into(),
                value: "窗口".into(),
            },
            LocalStep::ReadClipboard {
                variable: "clip".into(),
            },
            LocalStep::InputText {
                text: "${title}: ${clip} $$ ${title}".into(),
            },
        ]);
        local.validate().unwrap();
        let vars = BTreeMap::from([
            ("title".into(), "窗口".into()),
            ("clip".into(), "${unresolved}".into()),
        ]);
        assert_eq!(
            expand_template("${title}: ${clip} $$", &vars).unwrap(),
            "窗口: ${unresolved} $"
        );
        assert_eq!(expand_template("$${missing}", &vars).unwrap(), "${missing}");
        for text in ["${missing}", "${title", "${}", "${1bad}"] {
            assert!(expand_template(text, &vars).is_err());
        }
        assert!(action(vec![LocalStep::SetVariable {
            name: "self".into(),
            value: "${self}".into()
        }])
        .validate()
        .is_err());
    }

    #[test]
    fn limits_cover_steps_waits_variables_and_utf16_expansion() {
        assert!(action(vec![]).validate().is_err());
        assert!(
            action(vec![LocalStep::Delay { milliseconds: 0 }; MAX_STEPS])
                .validate()
                .is_ok()
        );
        assert!(
            action(vec![LocalStep::Delay { milliseconds: 0 }; MAX_STEPS + 1])
                .validate()
                .is_err()
        );
        assert!(action(vec![LocalStep::Delay { milliseconds: 5001 }])
            .validate()
            .is_err());
        assert!(action(vec![LocalStep::Delay { milliseconds: 5000 }; 2])
            .validate()
            .is_ok());
        assert!(action(vec![LocalStep::Delay { milliseconds: 4000 }; 3])
            .validate()
            .is_err());
        let steps = (0..17)
            .map(|index| LocalStep::ReadClipboard {
                variable: format!("v{index}"),
            })
            .collect();
        assert!(action(steps).validate().is_err());
        assert!(validate_text(&"😀".repeat(MAX_TEXT_UNITS / 2)).is_ok());
        assert!(validate_text(&"😀".repeat(MAX_TEXT_UNITS / 2 + 1)).is_err());
        let variables = BTreeMap::from([("value".into(), "a".repeat(MAX_TEXT_UNITS))]);
        assert!(expand_template("${value}x", &variables).is_err());
        assert!(action(vec![
            LocalStep::SetClipboard {
                text: "a".repeat(MAX_TEXT_UNITS)
            };
            4
        ])
        .validate()
        .is_err());
    }

    #[test]
    fn static_open_is_checked_and_dynamic_open_retains_runtime_check() {
        assert!(action(vec![LocalStep::Open {
            target: "relative.exe".into()
        }])
        .validate()
        .is_err());
        assert!(action(vec![
            LocalStep::SetVariable {
                name: "url".into(),
                value: "https://example.com".into()
            },
            LocalStep::Open {
                target: "${url}".into()
            },
        ])
        .validate()
        .is_ok());
        assert!(action(vec![
            LocalStep::SetVariable {
                name: "url".into(),
                value: "relative.exe".into()
            },
            LocalStep::Open {
                target: "${url}".into()
            },
        ])
        .validate()
        .is_err());
        assert!(action(vec![
            LocalStep::ReadClipboard {
                variable: "url".into()
            },
            LocalStep::Open {
                target: "${url}".into()
            },
        ])
        .validate()
        .is_ok());
    }

    #[test]
    fn urls_are_extracted_in_order_without_trailing_punctuation() {
        assert_eq!(
            extract_urls("见 https://a.com/x?q=1。另有 www.b.org, 和 https://a.com/x?q=1"),
            vec!["https://a.com/x?q=1", "https://www.b.org"]
        );
        assert_eq!(extract_urls("（http://c.net/p）"), vec!["http://c.net/p"]);
        assert!(extract_urls("没有网址 https:// ftp://x").is_empty());
        let many = (0..20)
            .map(|index| format!("https://e{index}.com"))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(extract_urls(&many).len(), MAX_OPEN_URLS);
    }

    #[test]
    fn search_url_encodes_trimmed_query_into_single_placeholder() {
        assert_eq!(
            search_url("https://www.google.com/search?q=%s", "  a b&c/中 ").unwrap(),
            "https://www.google.com/search?q=a%20b%26c%2F%E4%B8%AD"
        );
        assert_eq!(
            search_url("https://s.com/?q=%s", &"x".repeat(300))
                .unwrap()
                .len(),
            "https://s.com/?q=".len() + 128
        );
        for template in ["file:///%s", "https://s.com/?q=", "https://s.com/%s/%s"] {
            assert!(search_url(template, "a").is_err(), "{template}");
        }
        assert!(search_url("https://s.com/?q=%s", "  ").is_err());
    }

    #[test]
    fn app_name_suffix_is_removed_only_when_a_title_remains() {
        assert_eq!(
            strip_app_name("标题 - 子标题 - Google Chrome"),
            "标题 - 子标题"
        );
        assert_eq!(strip_app_name("main.rs — Code"), "main.rs");
        assert_eq!(strip_app_name(" - Chrome"), " - Chrome");
        assert_eq!(strip_app_name("无后缀"), "无后缀");
    }

    #[test]
    fn runtime_text_steps_define_variables_and_static_inputs_are_checked() {
        action(vec![
            LocalStep::CopySelection {
                variable: "text".into(),
            },
            LocalStep::ReadWindowTitle {
                variable: "title".into(),
                strip_app_name: true,
            },
            LocalStep::WebSearch {
                url: "https://www.google.com/search?q=%s".into(),
                query: "${text}".into(),
            },
            LocalStep::OpenUrls {
                text: "${title} ${text}".into(),
            },
        ])
        .validate()
        .unwrap();
        assert!(action(vec![LocalStep::OpenUrls {
            text: "没有网址".into()
        }])
        .validate()
        .is_err());
        assert!(action(vec![LocalStep::WebSearch {
            url: "https://s.com/?q=".into(),
            query: "a".into()
        }])
        .validate()
        .is_err());
        let json = r#"{"version":1,"steps":[{"kind":"read_window_title","variable":"t"}]}"#;
        let parsed: LocalAction = serde_json::from_str(json).unwrap();
        assert_eq!(
            parsed.steps[0],
            LocalStep::ReadWindowTitle {
                variable: "t".into(),
                strip_app_name: false
            }
        );
        assert_eq!(serde_json::to_string(&parsed).unwrap(), json);
    }

    #[test]
    fn unknown_fields_versions_and_step_kinds_are_rejected() {
        let parsed: LocalAction =
            serde_json::from_str(r#"{"version":1,"steps":[{"kind":"input_text","text":"hello"}]}"#)
                .unwrap();
        parsed.validate().unwrap();
        for json in [
            r#"{"version":1,"steps":[{"kind":"input_text","text":"hello","extra":1}]}"#,
            r#"{"version":1,"steps":[{"kind":"execute_code","code":"x"}]}"#,
            r#"{"version":1,"steps":[],"extra":1}"#,
        ] {
            assert!(serde_json::from_str::<LocalAction>(json).is_err());
        }
        assert!(LocalAction {
            version: 2,
            steps: parsed.steps
        }
        .validate()
        .is_err());
    }

    #[test]
    fn descriptions_do_not_include_text_or_open_targets() {
        for step in [
            LocalStep::InputText {
                text: "private".into(),
            },
            LocalStep::SetClipboard {
                text: "private".into(),
            },
            LocalStep::SetVariable {
                name: "safe_name".into(),
                value: "private".into(),
            },
            LocalStep::Open {
                target: "https://private.example".into(),
            },
        ] {
            assert!(!step.description().contains("private"));
        }
    }
}
