//! 应用持久化边界。纯核心无需了解文件格式或用户目录。
use omnitrigger_core::{
    Activation, Button, Engine, MouseButton, RegistrationError, TriggerSpec, WheelDirection,
};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    OnceLock,
};

use crate::execution::CommandAction;
use crate::plugins::PluginActionRef;
use crate::shortcut::ShortcutAction;
use crate::wheels::WheelActionRef;

static CUSTOM_PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn use_path(path: PathBuf) -> Result<(), String> {
    CUSTOM_PATH.set(path).map_err(|_| "配置路径已设置。".into())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub spec: TriggerSpec,
    pub message: String,
    pub enabled: bool,
    pub action: ActionSpec,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ActionSpec {
    Notification,
    ShowPanel,
    ShowWheel(WheelActionRef),
    Command(CommandAction),
    SendShortcut(ShortcutAction),
    Plugin(PluginActionRef),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    #[serde(rename = "version")]
    version: u32,
    #[serde(rename = "bindings")]
    bindings: Vec<BindingDto>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BindingDto {
    #[serde(rename = "id")]
    id: u64,
    #[serde(rename = "held")]
    held: Vec<ButtonDto>,
    #[serde(rename = "activation")]
    activation: ActivationDto,
    #[serde(rename = "message")]
    message: String,
    #[serde(rename = "enabled")]
    enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    action: Option<ActionSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    command: Option<CommandAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    show_panel: Option<bool>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum ButtonDto {
    Key(u16),
    Left,
    Right,
    Middle,
    X1,
    X2,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum ActivationDto {
    Press(ButtonDto),
    Hold { button: ButtonDto, duration_ms: u32 },
    WheelUp,
    WheelDown,
    WheelLeft,
    WheelRight,
}

impl From<Button> for ButtonDto {
    fn from(button: Button) -> Self {
        match button {
            Button::Key(code) => Self::Key(code),
            Button::Mouse(MouseButton::Left) => Self::Left,
            Button::Mouse(MouseButton::Right) => Self::Right,
            Button::Mouse(MouseButton::Middle) => Self::Middle,
            Button::Mouse(MouseButton::X1) => Self::X1,
            Button::Mouse(MouseButton::X2) => Self::X2,
        }
    }
}

impl From<ButtonDto> for Button {
    fn from(button: ButtonDto) -> Self {
        match button {
            ButtonDto::Key(code) => Self::Key(code),
            ButtonDto::Left => Self::Mouse(MouseButton::Left),
            ButtonDto::Right => Self::Mouse(MouseButton::Right),
            ButtonDto::Middle => Self::Mouse(MouseButton::Middle),
            ButtonDto::X1 => Self::Mouse(MouseButton::X1),
            ButtonDto::X2 => Self::Mouse(MouseButton::X2),
        }
    }
}

impl From<&Binding> for BindingDto {
    fn from(binding: &Binding) -> Self {
        Self {
            id: binding.spec.id,
            held: binding.spec.held.iter().copied().map(Into::into).collect(),
            activation: match binding.spec.activation {
                Activation::Press(button) => ActivationDto::Press(button.into()),
                Activation::Hold {
                    button,
                    duration_ms,
                } => ActivationDto::Hold {
                    button: button.into(),
                    duration_ms,
                },
                Activation::Wheel(WheelDirection::Up) => ActivationDto::WheelUp,
                Activation::Wheel(WheelDirection::Down) => ActivationDto::WheelDown,
                Activation::Wheel(WheelDirection::Left) => ActivationDto::WheelLeft,
                Activation::Wheel(WheelDirection::Right) => ActivationDto::WheelRight,
            },
            message: binding.message.clone(),
            enabled: binding.enabled,
            action: Some(binding.action.clone()),
            command: None,
            show_panel: None,
        }
    }
}

impl BindingDto {
    fn into_binding(self, version: u32) -> Result<Binding, String> {
        let action = if version == 5 {
            if self.command.is_some() || self.show_panel.is_some() {
                return Err("版本 5 配置不能混用旧版 command / show_panel 字段。".into());
            }
            self.action.ok_or("版本 5 绑定缺少动作描述。")?
        } else {
            if self.action.is_some() {
                return Err("旧版配置不能包含版本 5 动作描述。".into());
            }
            if self.show_panel == Some(true) && self.command.is_some() {
                return Err(format!(
                    "触发器 #{}：运行程序与打开触发面板不能同时启用。",
                    self.id
                ));
            }
            if self.show_panel == Some(true) {
                ActionSpec::ShowPanel
            } else if let Some(command) = self.command {
                ActionSpec::Command(command)
            } else {
                ActionSpec::Notification
            }
        };
        Ok(Binding {
            spec: TriggerSpec {
                id: self.id,
                held: self.held.into_iter().map(Into::into).collect(),
                activation: match self.activation {
                    ActivationDto::Press(button) => Activation::Press(button.into()),
                    ActivationDto::Hold {
                        button,
                        duration_ms,
                    } => Activation::Hold {
                        button: button.into(),
                        duration_ms,
                    },
                    ActivationDto::WheelUp => Activation::Wheel(WheelDirection::Up),
                    ActivationDto::WheelDown => Activation::Wheel(WheelDirection::Down),
                    ActivationDto::WheelLeft => Activation::Wheel(WheelDirection::Left),
                    ActivationDto::WheelRight => Activation::Wheel(WheelDirection::Right),
                },
            },
            message: self.message,
            enabled: self.enabled,
            action,
        })
    }
}

pub fn defaults() -> Vec<Binding> {
    vec![
        Binding {
            spec: TriggerSpec {
                id: 1,
                held: vec![Button::Key(0x14)],
                activation: Activation::Press(Button::Key(0x57)),
            },
            message: "CapsLock + W 已触发，OmniTrigger 工作正常。".into(),
            enabled: true,
            action: ActionSpec::Notification,
        },
        Binding {
            spec: TriggerSpec {
                id: 2,
                held: vec![Button::Mouse(MouseButton::X1)],
                activation: Activation::Wheel(WheelDirection::Up),
            },
            message: "鼠标侧键 + 滚轮上滚，组合触发成功。".into(),
            enabled: true,
            action: ActionSpec::Notification,
        },
        Binding {
            spec: TriggerSpec {
                id: 3,
                held: vec![Button::Mouse(MouseButton::X1)],
                activation: Activation::Wheel(WheelDirection::Down),
            },
            message: "鼠标侧键 + 滚轮下滚，组合触发成功。".into(),
            enabled: true,
            action: ActionSpec::Notification,
        },
        Binding {
            spec: TriggerSpec {
                id: 4,
                held: vec![Button::Key(0x14)],
                activation: Activation::Press(Button::Key(0x20)),
            },
            message: "打开触发面板".into(),
            enabled: true,
            action: ActionSpec::ShowPanel,
        },
        Binding {
            spec: TriggerSpec {
                id: 5,
                held: vec![],
                activation: Activation::Hold {
                    button: Button::Mouse(MouseButton::Right),
                    duration_ms: 300,
                },
            },
            message: "打开触发面板".into(),
            enabled: true,
            action: ActionSpec::ShowPanel,
        },
    ]
}

pub fn path() -> PathBuf {
    if let Some(path) = CUSTOM_PATH.get() {
        return path.clone();
    }
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("OmniTrigger")
        .join("bindings.json")
}

fn decode(text: &str) -> Result<Vec<Binding>, String> {
    let settings: Settings =
        serde_json::from_str(text).map_err(|e| format!("配置格式有误：{e}"))?;
    if !matches!(settings.version, 1..=5) {
        return Err("配置版本不受支持，原文件已保留。".into());
    }
    let bindings = settings
        .bindings
        .into_iter()
        .map(|binding| binding.into_binding(settings.version))
        .collect::<Result<Vec<_>, _>>()?;
    validate(&bindings)?;
    Ok(bindings)
}

/// 录制结果与已有配置的冲突。`other` 指向冲突的已有触发器，不改写任何一方。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    pub title: &'static str,
    pub detail: String,
    pub other: Option<u64>,
}

pub fn next_id(bindings: &[Binding]) -> u64 {
    bindings
        .iter()
        .map(|binding| binding.spec.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1)
}

fn bare_primary_click(spec: &TriggerSpec) -> bool {
    spec.held.is_empty()
        && matches!(
            spec.activation,
            Activation::Press(Button::Mouse(MouseButton::Left | MouseButton::Right))
        )
}

/// 在添加前按与注册相同的规则检查录制结果，返回可展示的结构化冲突。
pub fn check_candidate(bindings: &[Binding], spec: &TriggerSpec) -> Result<(), Conflict> {
    if bare_primary_click(spec) {
        return Err(Conflict {
            title: "不能单独使用",
            detail: "鼠标左键或右键请与其他键组合使用，以便仍可操作界面。".into(),
            other: None,
        });
    }
    let id = next_id(bindings);
    let mut specs: Vec<_> = bindings.iter().map(|b| b.spec.clone()).collect();
    specs.push(TriggerSpec { id, ..spec.clone() });
    let error = match Engine::new(specs) {
        Ok(_) => return Ok(()),
        Err(error) => error,
    };
    let pick = |a: u64, b: u64| Some(if a == id { b } else { a });
    Err(match error {
        RegistrationError::DuplicateTrigger { first, second } => Conflict {
            title: "与已有触发器重复",
            detail: "该组合已注册，不会覆盖原有绑定。".into(),
            other: pick(first, second),
        },
        RegistrationError::TerminalIsPrefix {
            terminal_binding,
            prefix_binding,
        } => Conflict {
            title: "与已有触发器的前缀冲突",
            detail: if terminal_binding == id {
                "该组合的目标键是另一个触发器的前缀，按住它时无法区分两者。".into()
            } else {
                "该组合的前缀是另一个触发器的目标键，按住它时无法区分两者。".into()
            },
            other: pick(terminal_binding, prefix_binding),
        },
        other => Conflict {
            title: "无法添加",
            detail: other.to_string(),
            other: None,
        },
    })
}

pub fn validate(bindings: &[Binding]) -> Result<(), String> {
    // 禁用项也需要有效、唯一，以免启用时才发现存储中存在冲突。
    Engine::new(bindings.iter().map(|b| b.spec.clone()).collect()).map_err(|e| e.to_string())?;
    if bindings.iter().any(|b| bare_primary_click(&b.spec)) {
        return Err("鼠标左键或右键请与其他键组合使用，以便仍可操作界面。".into());
    }
    if bindings
        .iter()
        .any(|b| b.message.trim().is_empty() || b.message.chars().count() > 2000)
    {
        return Err("提示消息应包含 1 至 2000 个字符。".into());
    }
    for binding in bindings {
        let result = match &binding.action {
            ActionSpec::Command(command) => command.validate(),
            ActionSpec::SendShortcut(shortcut) => shortcut.validate(),
            ActionSpec::ShowWheel(reference) => reference.validate(),
            ActionSpec::Plugin(reference) => {
                reference.validate().map_err(|error| error.to_string())
            }
            ActionSpec::Notification | ActionSpec::ShowPanel => Ok(()),
        };
        if let Err(error) = result {
            return Err(format!("触发器 #{}：{error}", binding.spec.id));
        }
    }
    Ok(())
}

pub fn load() -> Result<Vec<Binding>, String> {
    match std::fs::read_to_string(path()) {
        Ok(text) => decode(&text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(defaults()),
        Err(error) => Err(format!("读取配置失败：{error}")),
    }
}

pub fn save(bindings: &[Binding]) -> Result<(), String> {
    save_to(&path(), bindings)
}

fn save_to(destination: &Path, bindings: &[Binding]) -> Result<(), String> {
    validate(bindings)?;
    let text = serde_json::to_vec_pretty(&Settings {
        version: 5,
        bindings: bindings.iter().map(Into::into).collect(),
    })
    .map_err(|e| e.to_string())?;
    match std::fs::read_to_string(destination) {
        Ok(original) => {
            // 不覆盖损坏或未来版本的配置；用户需要先明确修复原文件。
            decode(&original)?;
            let settings: Settings =
                serde_json::from_str(&original).map_err(|error| error.to_string())?;
            if settings.version < 5 {
                backup_legacy(destination, settings.version, original.as_bytes())?;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("读取原配置失败，原文件已保留：{error}")),
    }
    atomic_write(destination, &text)
}

fn backup_legacy(destination: &Path, version: u32, original: &[u8]) -> Result<(), String> {
    let file_name = destination.file_name().ok_or("配置路径不可用")?;
    let mut backup_name = file_name.to_os_string();
    backup_name.push(format!(".v{version}.bak"));
    let backup = destination.with_file_name(backup_name);
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&backup)
    {
        Ok(mut file) => {
            if let Err(error) = file.write_all(original).and_then(|_| file.sync_all()) {
                drop(file);
                let _ = std::fs::remove_file(&backup);
                return Err(format!("备份旧版配置失败，原文件已保留：{error}"));
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if std::fs::read(&backup).ok().as_deref() == Some(original) {
                Ok(())
            } else {
                Err(format!(
                    "旧版备份已存在且内容不同，未覆盖：{}",
                    backup.display()
                ))
            }
        }
        Err(error) => Err(format!("备份旧版配置失败，原文件已保留：{error}")),
    }
}

/// 在同目录写完整临时文件后替换；失败不会删除现有配置。
pub(crate) fn atomic_write(destination: &Path, contents: &[u8]) -> Result<(), String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|error| format!("创建配置目录失败：{error}"))?;
    let mut temporary_name = destination
        .file_name()
        .ok_or("配置路径不可用")?
        .to_os_string();
    temporary_name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let temporary = parent.join(temporary_name);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| format!("创建临时配置失败：{error}"))?;
    let result = file.write_all(contents).and_then(|_| file.sync_all());
    drop(file);
    let result = result.and_then(|_| std::fs::rename(&temporary, destination));
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("保存配置失败，原文件已保留：{error}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn encode(bindings: &[Binding]) -> String {
        serde_json::to_string(&Settings {
            version: 5,
            bindings: bindings.iter().map(Into::into).collect(),
        })
        .unwrap()
    }

    fn legacy(version: u32, command: bool, panel: bool) -> String {
        let mut binding = json!({
            "id": 42,
            "held": [{"kind":"key", "value":20}],
            "activation": {"kind":"press", "value":{"kind":"key", "value":87}},
            "message": "中文 名称",
            "enabled": false
        });
        if command {
            binding["command"] = json!({
                "program": "C:\\截图 工具\\raishot-cli.exe",
                "args": ["--capture", "中文 空格 & 符号"],
                "working_directory": "C:\\截图 工具"
            });
        }
        if panel {
            binding["show_panel"] = true.into();
        }
        json!({"version":version, "bindings":[binding]}).to_string()
    }

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "omnitrigger-settings-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn config(&self) -> PathBuf {
            self.0.join("bindings.json")
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn navigation_example_consumes_whole_press_and_maps_to_expected_keys() {
        use omnitrigger_core::{InputEvent, InputKind};
        let bindings = decode(include_str!("../../../examples/capslock-navigation.json")).unwrap();
        let expected = [
            (0x4f, 0x08),
            (0x49, 0x26),
            (0x4a, 0x25),
            (0x4b, 0x28),
            (0x4c, 0x27),
        ];
        assert_eq!(bindings.len(), expected.len());
        let mut engine = Engine::new(bindings.iter().map(|b| b.spec.clone()).collect()).unwrap();
        let event = |kind| InputEvent {
            kind,
            injected: false,
        };
        for (binding, (input, output)) in bindings.iter().zip(expected) {
            assert_eq!(
                binding.action,
                ActionSpec::SendShortcut(ShortcutAction { keys: vec![output] })
            );
            let caps = InputKind::Down(Button::Key(0x14));
            assert!(engine.process(event(caps)).consume);
            let press = InputKind::Down(Button::Key(input));
            let hit = engine.process(event(press));
            assert!(hit.consume && hit.replay.is_empty());
            assert_eq!(hit.triggered, vec![binding.spec.id]);
            assert!(engine
                .process(InputEvent {
                    kind: press,
                    injected: true
                })
                .triggered
                .is_empty());
            assert!(engine.process(event(press)).triggered.is_empty());
            for key in [input, 0x14] {
                let release = engine.process(event(InputKind::Up(Button::Key(key))));
                assert!(
                    release.consume && release.replay.is_empty() && release.triggered.is_empty()
                );
            }
        }
    }

    #[test]
    fn shipped_right_hold_example_loads_as_a_panel_trigger() {
        let bindings = decode(include_str!("../../../examples/right-hold-panel.json")).unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].action, ActionSpec::ShowPanel);
        assert_eq!(
            bindings[0].spec.activation,
            Activation::Hold {
                button: Button::Mouse(MouseButton::Right),
                duration_ms: 300
            }
        );
    }

    #[test]
    fn v5_round_trip_preserves_all_action_variants_and_unavailable_plugin_parameters() {
        let mut expected = defaults();
        expected[1].enabled = false;
        expected[1].action = ActionSpec::Command(CommandAction {
            program: "C:\\截图 工具\\raishot-cli.exe".into(),
            args: vec!["--capture".into(), "带 空格 & 符号".into()],
            working_directory: Some("C:\\截图 工具".into()),
        });
        let mut missing = PluginActionRef::raishot_capture();
        missing.plugin_id = "missing-plugin".into();
        missing
            .parameters
            .insert("保留参数".into(), json!({"nested": [true, 42]}));
        expected[2].action = ActionSpec::Plugin(missing);
        expected[4].action =
            ActionSpec::SendShortcut(ShortcutAction::parse("RightCtrl+Backspace").unwrap());
        assert_eq!(decode(&encode(&expected)).unwrap(), expected);
        let value: serde_json::Value = serde_json::from_str(&encode(&expected)).unwrap();
        assert!(value["bindings"][1].get("command").is_none());
        assert!(value["bindings"][4].get("show_panel").is_none());
        assert_eq!(value["bindings"][2]["action"]["pluginId"], "missing-plugin");
        assert_eq!(
            value["bindings"][4]["action"],
            json!({"kind":"send_shortcut", "keys":[163, 8]})
        );
    }

    #[test]
    fn shortcut_configuration_rejects_invalid_key_order_and_unknown_parameters() {
        let mut value: serde_json::Value = serde_json::from_str(&encode(&defaults())).unwrap();
        for keys in [json!([]), json!([0x43, 0xa2]), json!([0xa2, 0xa3, 0x43])] {
            value["bindings"][0]["action"] = json!({"kind":"send_shortcut", "keys": keys});
            assert!(decode(&value.to_string()).is_err());
        }
        value["bindings"][0]["action"] = json!({"kind":"send_shortcut", "keys":[8], "repeat":true});
        assert!(decode(&value.to_string()).is_err());
        value["bindings"][0]["action"] = json!({"kind":"send_shortcut", "keys":[8]});
        let actual = decode(&value.to_string()).unwrap();
        assert_eq!(
            actual[0].action,
            ActionSpec::SendShortcut(ShortcutAction { keys: vec![8] })
        );
    }

    #[test]
    fn wheel_references_preserve_missing_ids_but_reject_invalid_or_unknown_fields() {
        let mut expected = defaults();
        expected[0].action = ActionSpec::ShowWheel(WheelActionRef {
            wheel_id: "missing-wheel_2".into(),
        });
        let text = encode(&expected);
        assert_eq!(decode(&text).unwrap(), expected);
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            value["bindings"][0]["action"],
            json!({"kind":"show_wheel", "wheel_id":"missing-wheel_2"})
        );
        value["bindings"][0]["action"]["wheel_id"] = "../invalid".into();
        assert!(decode(&value.to_string()).is_err());
        value["bindings"][0]["action"]["wheel_id"] = "orbit-edit".into();
        value["bindings"][0]["action"]["unknown"] = true.into();
        assert!(decode(&value.to_string()).is_err());
    }

    #[test]
    fn every_legacy_version_preserves_notification_command_panel_and_identity() {
        for version in 1..=4 {
            for (command, panel) in [(false, false), (true, false), (false, true)] {
                let actual = decode(&legacy(version, command, panel)).unwrap();
                assert_eq!(actual.len(), 1);
                assert_eq!(actual[0].spec.id, 42);
                assert_eq!(actual[0].message, "中文 名称");
                assert!(!actual[0].enabled);
                match (&actual[0].action, command, panel) {
                    (ActionSpec::Notification, false, false)
                    | (ActionSpec::ShowPanel, false, true) => {}
                    (ActionSpec::Command(action), true, false) => {
                        assert_eq!(action.args, ["--capture", "中文 空格 & 符号"])
                    }
                    _ => panic!("旧配置迁移改写了动作"),
                }
                assert_eq!(decode(&encode(&actual)).unwrap(), actual);
            }
        }
    }

    #[test]
    fn incompatible_and_ambiguous_configurations_are_rejected() {
        assert!(decode(r#"{"version":6,"bindings":[]}"#).is_err());
        assert!(decode(&legacy(4, true, true)).is_err());
        let mut value: serde_json::Value = serde_json::from_str(&legacy(4, false, false)).unwrap();
        value["version"] = 5.into();
        assert!(decode(&value.to_string()).is_err());
        value["bindings"][0]["action"] = json!({"kind":"notification"});
        value["bindings"][0]["show_panel"] = false.into();
        assert!(decode(&value.to_string()).is_err());
        value["bindings"][0]
            .as_object_mut()
            .unwrap()
            .remove("show_panel");
        value["bindings"][0]["action"] = json!({"kind":"future-action"});
        assert!(decode(&value.to_string()).is_err());
    }

    #[test]
    fn save_backs_up_legacy_bytes_once_then_saves_version_five() {
        let scratch = Scratch::new();
        let destination = scratch.config();
        let original = legacy(4, true, false);
        std::fs::write(&destination, &original).unwrap();
        let mut bindings = decode(&original).unwrap();
        bindings[0].message = "修改后".into();
        save_to(&destination, &bindings).unwrap();
        assert_eq!(
            std::fs::read_to_string(scratch.0.join("bindings.json.v4.bak")).unwrap(),
            original
        );
        assert_eq!(
            decode(&std::fs::read_to_string(&destination).unwrap()).unwrap(),
            bindings
        );
        bindings[0].enabled = true;
        save_to(&destination, &bindings).unwrap();
        assert_eq!(
            std::fs::read_to_string(scratch.0.join("bindings.json.v4.bak")).unwrap(),
            original
        );
        assert_eq!(
            decode(&std::fs::read_to_string(destination).unwrap()).unwrap(),
            bindings
        );
    }

    #[test]
    fn backup_failure_or_invalid_original_keeps_original_configuration_unchanged() {
        let scratch = Scratch::new();
        let destination = scratch.config();
        let original = legacy(4, false, false);
        std::fs::write(&destination, &original).unwrap();
        std::fs::create_dir(scratch.0.join("bindings.json.v4.bak")).unwrap();
        assert!(save_to(&destination, &defaults()).is_err());
        assert_eq!(std::fs::read_to_string(&destination).unwrap(), original);
        std::fs::write(&destination, "invalid configuration").unwrap();
        assert!(save_to(&destination, &defaults()).is_err());
        assert_eq!(
            std::fs::read_to_string(destination).unwrap(),
            "invalid configuration"
        );
    }

    #[test]
    fn default_hold_binding_is_valid_and_preserves_input_serialization() {
        let bindings = defaults();
        validate(&bindings).unwrap();
        let hold = &bindings[4];
        assert_eq!(hold.spec.id, 5);
        assert!(hold.enabled && hold.action == ActionSpec::ShowPanel);
        assert!(hold.spec.held.is_empty());
        assert_eq!(
            serde_json::to_value(BindingDto::from(hold)).unwrap()["activation"],
            json!({
                "kind":"hold", "value":{"button":{"kind":"right"}, "duration_ms":300}
            })
        );
    }

    #[test]
    fn invalid_hold_duration_and_duplicate_recordings_are_rejected_without_rewriting() {
        let mut bindings = defaults();
        bindings[4].spec.activation = Activation::Hold {
            button: Button::Mouse(MouseButton::Right),
            duration_ms: 49,
        };
        assert!(decode(&encode(&bindings)).is_err());
        assert_eq!(
            bindings[4].spec.activation,
            Activation::Hold {
                button: Button::Mouse(MouseButton::Right),
                duration_ms: 49
            }
        );
        let mut bindings = defaults();
        let mut duplicate = bindings[0].clone();
        duplicate.spec.id = 42;
        bindings.push(duplicate);
        assert!(validate(&bindings).is_err());
        bindings.pop();
        bindings[0].message = " ".into();
        assert!(validate(&bindings).is_err());
    }

    #[test]
    fn candidate_conflicts_include_disabled_and_unavailable_bindings() {
        let mut values = defaults();
        values[0].enabled = false;
        let mut missing = PluginActionRef::raishot_capture();
        missing.plugin_id = "missing".into();
        values[0].action = ActionSpec::Plugin(missing);
        let duplicate = check_candidate(&values, &values[0].spec).unwrap_err();
        assert_eq!(duplicate.other, Some(1));
        assert_eq!(duplicate.title, "与已有触发器重复");
        let prefix = TriggerSpec {
            id: 0,
            held: vec![Button::Key(0xa0)],
            activation: Activation::Press(Button::Key(0x14)),
        };
        assert_eq!(
            check_candidate(&values, &prefix).unwrap_err().other,
            Some(1)
        );
        let bare_left = TriggerSpec {
            id: 0,
            held: vec![],
            activation: Activation::Press(Button::Mouse(MouseButton::Left)),
        };
        assert_eq!(
            check_candidate(&values, &bare_left).unwrap_err().other,
            None
        );
        let fresh = TriggerSpec {
            id: 0,
            held: vec![Button::Key(0x14)],
            activation: Activation::Press(Button::Key(0x45)),
        };
        assert!(check_candidate(&values, &fresh).is_ok());
        assert_eq!(values.len(), 5);
    }
}
