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
    SendShortcut { keys: Vec<u16> },
    KeyTap { key: u16 },
    KeySequence { modifiers: Vec<u16>, keys: Vec<u16> },
    InputText { text: String },
    SetClipboard { text: String },
    ReadClipboard { variable: String },
    SetVariable { name: String, value: String },
    Open { target: String },
    Delay { milliseconds: u32 },
}

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
                    LocalStep::ReadClipboard { variable } => {
                        define_variable(variable, None, &mut variables)?;
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
        }
    }
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
