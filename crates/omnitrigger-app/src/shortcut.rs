//! 发送快捷键的配置与文本输入；实际注入由平台适配层执行。
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShortcutAction {
    /// 按顺序保存修饰键，最后一项为目标键；不改写左右键或输入顺序。
    pub keys: Vec<u16>,
    #[serde(default, skip_serializing_if = "KeyboardTiming::is_default")]
    pub timing: KeyboardTiming,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyboardTiming {
    #[serde(default, skip_serializing_if = "is_zero")]
    pub key_down_delay_ms: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub process_overrides: BTreeMap<String, u32>,
}
fn is_zero(value: &u32) -> bool {
    *value == 0
}
fn process_key(name: &str) -> Result<String, String> {
    if name.is_empty()
        || name.len() > 260
        || name.trim() != name
        || name
            .chars()
            .any(|c| c.is_control() || "/\\:*?;\"<>|".contains(c))
    {
        return Err("时序覆盖需要明确进程文件名，不能使用路径或通配符。".into());
    }
    let canonical = name.to_lowercase();
    let canonical = canonical
        .strip_suffix(".exe")
        .unwrap_or(&canonical)
        .to_owned();
    if canonical.is_empty() || canonical == "." || canonical == ".." {
        return Err("时序覆盖的进程名无效。".into());
    }
    Ok(canonical)
}
impl KeyboardTiming {
    pub fn is_default(&self) -> bool {
        self.key_down_delay_ms == 0 && self.process_overrides.is_empty()
    }
    pub fn validate(&self) -> Result<(), String> {
        omnitrigger_windows::validate_key_timing(1, self.key_down_delay_ms)?;
        if self.process_overrides.len() > 16 {
            return Err("按键时序最多设置16个进程覆盖。".into());
        }
        let mut seen = BTreeSet::new();
        for (name, delay) in &self.process_overrides {
            if !seen.insert(process_key(name)?) {
                return Err("按键时序包含重复进程名。".into());
            }
            omnitrigger_windows::validate_key_timing(1, *delay)?;
        }
        Ok(())
    }
    pub fn resolve(&self, process: &str) -> Result<u32, String> {
        self.validate()?;
        let key = process_key(process)?;
        Ok(self
            .process_overrides
            .iter()
            .find_map(|(name, delay)| {
                (process_key(name).ok().as_ref() == Some(&key)).then_some(*delay)
            })
            .unwrap_or(self.key_down_delay_ms))
    }
}

impl ShortcutAction {
    pub fn new(keys: Vec<u16>) -> Self {
        Self {
            keys,
            ..Default::default()
        }
    }
    pub fn edit_keys(&self, text: &str) -> Result<Self, String> {
        let mut action = self.clone();
        action.keys = Self::parse(text)?.keys;
        action.validate()?;
        Ok(action)
    }
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > 256 {
            return Err("快捷键不能超过 256 字节。".into());
        }
        let mut keys = Vec::new();
        for part in text.split('+') {
            let name = part.trim();
            if name.is_empty() {
                return Err("请填写一个按键或快捷键，例如 Backspace、Left、Ctrl+C。".into());
            }
            let key = parse_key(name).ok_or_else(|| format!("无法识别按键“{name}”。"))?;
            keys.push(key);
        }
        let shortcut = Self::new(keys);
        shortcut.validate()?;
        Ok(shortcut)
    }

    pub fn validate(&self) -> Result<(), String> {
        omnitrigger_windows::validate_shortcut(&self.keys)?;
        self.timing.validate()
    }

    pub fn display(&self) -> String {
        self.keys
            .iter()
            .copied()
            .map(key_name)
            .collect::<Vec<_>>()
            .join("+")
    }
}

fn parse_key(name: &str) -> Option<u16> {
    let name = name
        .chars()
        .filter(|character| !character.is_ascii_whitespace() && *character != '_')
        .collect::<String>()
        .to_lowercase();
    let named = match name.as_str() {
        "ctrl" | "control" | "leftctrl" | "lctrl" | "leftcontrol" | "lcontrol" | "左ctrl"
        | "左control" | "左控制键" => 0xa2,
        "rightctrl" | "rctrl" | "rightcontrol" | "rcontrol" | "右ctrl" | "右control"
        | "右控制键" => 0xa3,
        "shift" | "leftshift" | "lshift" | "左shift" | "左上档键" => 0xa0,
        "rightshift" | "rshift" | "右shift" | "右上档键" => 0xa1,
        "alt" | "leftalt" | "lalt" | "左alt" => 0xa4,
        "rightalt" | "ralt" | "右alt" => 0xa5,
        "win" | "windows" | "leftwin" | "lwin" | "左win" | "左windows" => 0x5b,
        "rightwin" | "rwin" | "右win" | "右windows" => 0x5c,
        "backspace" | "back" | "bs" | "退格" | "退格键" => 0x08,
        "tab" | "制表" | "制表键" => 0x09,
        "enter" | "return" | "回车" | "回车键" => 0x0d,
        "pause" | "break" => 0x13,
        "capslock" | "capital" | "大写锁定" => 0x14,
        "esc" | "escape" | "退出键" => 0x1b,
        "space" | "spacebar" | "空格" | "空格键" => 0x20,
        "pageup" | "pgup" | "向上翻页" => 0x21,
        "pagedown" | "pgdn" | "pgdown" | "向下翻页" => 0x22,
        "end" | "行尾" => 0x23,
        "home" | "行首" => 0x24,
        "left" | "arrowleft" | "左" | "左方向键" | "←" => 0x25,
        "up" | "arrowup" | "上" | "上方向键" | "↑" => 0x26,
        "right" | "arrowright" | "右" | "右方向键" | "→" => 0x27,
        "down" | "arrowdown" | "下" | "下方向键" | "↓" => 0x28,
        "printscreen" | "prtsc" | "prtscr" | "截图键" => 0x2c,
        "insert" | "ins" | "插入" => 0x2d,
        "delete" | "del" | "删除" | "删除键" => 0x2e,
        "apps" | "menu" | "菜单键" => 0x5d,
        "numpadmultiply" | "multiply" => 0x6a,
        "numpadadd" | "add" => 0x6b,
        "numpadsubtract" | "subtract" => 0x6d,
        "numpaddecimal" | "decimal" => 0x6e,
        "numpaddivide" | "divide" => 0x6f,
        "numlock" => 0x90,
        "scrolllock" => 0x91,
        "volumeup" | "音量加" | "音量增加" | "增大音量" => 0xaf,
        "volumedown" | "音量减" | "音量减少" | "减小音量" => 0xae,
        "volumemute" | "mute" | "静音" => 0xad,
        "semicolon" | ";" => 0xba,
        "plus" | "equals" | "=" => 0xbb,
        "comma" | "," => 0xbc,
        "minus" | "-" => 0xbd,
        "period" | "dot" | "." => 0xbe,
        "slash" | "/" => 0xbf,
        "backtick" | "grave" | "`" => 0xc0,
        "leftbracket" | "[" => 0xdb,
        "backslash" | "\\" => 0xdc,
        "rightbracket" | "]" => 0xdd,
        "quote" | "apostrophe" | "'" => 0xde,
        _ => {
            if name.len() == 1 {
                let character = name.as_bytes()[0];
                if character.is_ascii_alphanumeric() {
                    return Some(character.to_ascii_uppercase().into());
                }
            }
            if let Some(number) = name.strip_prefix('f').and_then(|n| n.parse::<u16>().ok()) {
                if (1..=24).contains(&number) {
                    return Some(0x6f + number);
                }
            }
            if let Some(number) = name
                .strip_prefix("numpad")
                .and_then(|n| n.parse::<u16>().ok())
            {
                if number <= 9 {
                    return Some(0x60 + number);
                }
            }
            // 显式虚拟键写法供配置中的其他系统键读回，不依赖本机键盘布局名称。
            return name
                .strip_prefix("vk")
                .and_then(|hex| u16::from_str_radix(hex.trim_start_matches("0x"), 16).ok());
        }
    };
    Some(named)
}

fn key_name(key: u16) -> String {
    match key {
        0xa2 => "Ctrl",
        0xa3 => "RightCtrl",
        0xa0 => "Shift",
        0xa1 => "RightShift",
        0xa4 => "Alt",
        0xa5 => "RightAlt",
        0x5b => "Win",
        0x5c => "RightWin",
        0x08 => "Backspace",
        0x09 => "Tab",
        0x0d => "Enter",
        0x13 => "Pause",
        0x14 => "CapsLock",
        0x1b => "Esc",
        0x20 => "Space",
        0x21 => "PageUp",
        0x22 => "PageDown",
        0x23 => "End",
        0x24 => "Home",
        0x25 => "Left",
        0x26 => "Up",
        0x27 => "Right",
        0x28 => "Down",
        0x2c => "PrintScreen",
        0x2d => "Insert",
        0x2e => "Delete",
        0x5d => "Apps",
        0x6a => "NumpadMultiply",
        0x6b => "NumpadAdd",
        0x6d => "NumpadSubtract",
        0x6e => "NumpadDecimal",
        0x6f => "NumpadDivide",
        0x90 => "NumLock",
        0x91 => "ScrollLock",
        0xad => "VolumeMute",
        0xae => "VolumeDown",
        0xaf => "VolumeUp",
        0xba => "Semicolon",
        0xbb => "Plus",
        0xbc => "Comma",
        0xbd => "Minus",
        0xbe => "Period",
        0xbf => "Slash",
        0xc0 => "Backtick",
        0xdb => "LeftBracket",
        0xdc => "Backslash",
        0xdd => "RightBracket",
        0xde => "Quote",
        0x30..=0x39 | 0x41..=0x5a => return char::from_u32(key.into()).unwrap().into(),
        0x60..=0x69 => return format!("Numpad{}", key - 0x60),
        0x70..=0x87 => return format!("F{}", key - 0x6f),
        _ => return format!("VK_{key:02X}"),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_preserves_explicit_keys_and_editor_changes_keep_process_overrides() {
        let mut action = ShortcutAction::parse("RightCtrl+RightAlt+RightShift+A").unwrap();
        action.timing = KeyboardTiming {
            key_down_delay_ms: 5,
            process_overrides: BTreeMap::from([("WPS.exe".into(), 10), ("工具".into(), 20)]),
        };
        assert_eq!(action.keys, vec![163, 165, 161, 65]);
        assert_eq!(action.timing.resolve("wps.EXE").unwrap(), 10);
        assert_eq!(action.timing.resolve("工具.exe").unwrap(), 20);
        assert_eq!(action.timing.resolve("editor.exe").unwrap(), 5);
        let edited = action.edit_keys("RightCtrl+B").unwrap();
        assert_eq!(edited.keys, vec![163, 66]);
        assert_eq!(edited.timing, action.timing);
        assert!(action.edit_keys("Ctrl+Ctrl+B").is_err());
        assert_eq!(action.keys, vec![163, 165, 161, 65]);
        let encoded = serde_json::to_string(&action).unwrap();
        assert_eq!(
            serde_json::from_str::<ShortcutAction>(&encoded).unwrap(),
            action
        );
    }

    #[test]
    fn timing_rejects_alias_duplicates_unknown_fields_and_unbounded_delays() {
        for timing in [
            KeyboardTiming {
                key_down_delay_ms: 1001,
                ..Default::default()
            },
            KeyboardTiming {
                process_overrides: BTreeMap::from([("Code".into(), 10), ("code.exe".into(), 20)]),
                ..Default::default()
            },
            KeyboardTiming {
                process_overrides: BTreeMap::from([("C:/code.exe".into(), 10)]),
                ..Default::default()
            },
            KeyboardTiming {
                process_overrides: BTreeMap::from([(".exe".into(), 10)]),
                ..Default::default()
            },
        ] {
            assert!(timing.validate().is_err());
        }
        assert!(
            serde_json::from_str::<ShortcutAction>(r#"{"keys":[65],"timing":{"delay":10}}"#)
                .is_err()
        );
        let default = ShortcutAction::new(vec![65]);
        assert_eq!(
            serde_json::to_value(default).unwrap(),
            serde_json::json!({"keys":[65]})
        );
    }

    #[test]
    fn accepts_single_edit_keys_and_shortcuts_without_changing_order_or_side() {
        for (text, keys) in [
            ("Backspace", vec![0x08]),
            ("←", vec![0x25]),
            ("Ctrl + Shift + C", vec![0xa2, 0xa0, 0x43]),
            ("右 Alt + F24", vec![0xa5, 0x87]),
            ("Win+Numpad0", vec![0x5b, 0x60]),
            ("Ctrl+Plus", vec![0xa2, 0xbb]),
            ("VolumeUp", vec![0xaf]),
            ("音量减少", vec![0xae]),
            ("静音", vec![0xad]),
        ] {
            let action = ShortcutAction::parse(text).unwrap();
            assert_eq!(action.keys, keys);
            assert_eq!(ShortcutAction::parse(&action.display()).unwrap(), action);
        }
    }

    #[test]
    fn rejects_ambiguous_or_malformed_shortcuts() {
        for text in [
            "",
            "Ctrl++C",
            "Ctrl+",
            "Ctrl",
            "C+Ctrl",
            "C+D",
            "Ctrl+LeftCtrl+C",
            "Ctrl+RightCtrl+C",
            "F25",
            "Numpad10",
            "unknown",
            "VK_00",
            "VK_FFFF",
        ] {
            assert!(ShortcutAction::parse(text).is_err(), "意外接纳 {text}");
        }
    }

    #[test]
    fn serialization_preserves_numeric_key_identity_and_rejects_extra_fields() {
        let action = ShortcutAction::parse("RightCtrl+Backspace").unwrap();
        let text = serde_json::to_string(&action).unwrap();
        assert_eq!(text, r#"{"keys":[163,8]}"#);
        assert_eq!(
            serde_json::from_str::<ShortcutAction>(&text).unwrap(),
            action
        );
        assert!(serde_json::from_str::<ShortcutAction>(r#"{"keys":[8],"repeat":true}"#).is_err());
    }

    #[test]
    fn every_supported_virtual_key_can_be_displayed_and_parsed_without_losing_identity() {
        for key in 0..=0xff {
            let action = ShortcutAction::new(vec![key]);
            if action.validate().is_ok() {
                assert_eq!(ShortcutAction::parse(&action.display()).unwrap(), action);
            }
        }
    }
}
