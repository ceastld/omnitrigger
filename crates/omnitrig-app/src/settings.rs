//! 应用持久化边界。纯核心无需了解文件格式或用户目录。
use omnitrig_core::{Activation, Button, Engine, MouseButton, TriggerSpec, WheelDirection};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone)]
pub struct Binding {
    pub spec: TriggerSpec,
    pub message: String,
    pub enabled: bool,
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
                Activation::Wheel(WheelDirection::Up) => ActivationDto::WheelUp,
                Activation::Wheel(WheelDirection::Down) => ActivationDto::WheelDown,
                Activation::Wheel(WheelDirection::Left) => ActivationDto::WheelLeft,
                Activation::Wheel(WheelDirection::Right) => ActivationDto::WheelRight,
            },
            message: binding.message.clone(),
            enabled: binding.enabled,
        }
    }
}

impl From<BindingDto> for Binding {
    fn from(binding: BindingDto) -> Self {
        Self {
            spec: TriggerSpec {
                id: binding.id,
                held: binding.held.into_iter().map(Into::into).collect(),
                activation: match binding.activation {
                    ActivationDto::Press(button) => Activation::Press(button.into()),
                    ActivationDto::WheelUp => Activation::Wheel(WheelDirection::Up),
                    ActivationDto::WheelDown => Activation::Wheel(WheelDirection::Down),
                    ActivationDto::WheelLeft => Activation::Wheel(WheelDirection::Left),
                    ActivationDto::WheelRight => Activation::Wheel(WheelDirection::Right),
                },
            },
            message: binding.message,
            enabled: binding.enabled,
        }
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
            message: "CapsLock + W 已触发，OmniTrig 工作正常。".into(),
            enabled: true,
        },
        Binding {
            spec: TriggerSpec {
                id: 2,
                held: vec![Button::Mouse(MouseButton::X1)],
                activation: Activation::Wheel(WheelDirection::Up),
            },
            message: "鼠标侧键 + 滚轮上滚，组合触发成功。".into(),
            enabled: true,
        },
        Binding {
            spec: TriggerSpec {
                id: 3,
                held: vec![Button::Mouse(MouseButton::X1)],
                activation: Activation::Wheel(WheelDirection::Down),
            },
            message: "鼠标侧键 + 滚轮下滚，组合触发成功。".into(),
            enabled: true,
        },
    ]
}

pub fn path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("OmniTrig")
        .join("bindings.json")
}

fn decode(text: &str) -> Result<Vec<Binding>, String> {
    let settings: Settings =
        serde_json::from_str(text).map_err(|e| format!("配置格式有误：{e}"))?;
    if settings.version != 1 {
        return Err("配置版本不受支持，原文件已保留。".into());
    }
    let bindings: Vec<Binding> = settings.bindings.into_iter().map(Into::into).collect();
    validate(&bindings)?;
    Ok(bindings)
}

pub fn validate(bindings: &[Binding]) -> Result<(), String> {
    // 禁用项也需要有效、唯一，以免启用时才发现存储中存在冲突。
    Engine::new(bindings.iter().map(|b| b.spec.clone()).collect()).map_err(|e| e.to_string())?;
    if bindings.iter().any(|b| {
        b.spec.held.is_empty()
            && matches!(
                b.spec.activation,
                Activation::Press(Button::Mouse(MouseButton::Left | MouseButton::Right))
            )
    }) {
        return Err("鼠标左键或右键请与其他键组合使用，以便仍可操作界面。".into());
    }
    if bindings
        .iter()
        .any(|b| b.message.trim().is_empty() || b.message.chars().count() > 2000)
    {
        return Err("提示消息应包含 1 至 2000 个字符。".into());
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
    validate(bindings)?;
    let destination = path();
    let parent = destination.parent().ok_or("配置路径不可用")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败：{e}"))?;
    let text = serde_json::to_string_pretty(&Settings {
        version: 1,
        bindings: bindings.iter().map(Into::into).collect(),
    })
    .map_err(|e| e.to_string())?;
    let temporary = parent.join(format!("bindings.{}.tmp", std::process::id()));
    std::fs::write(&temporary, text)
        .and_then(|_| std::fs::rename(&temporary, &destination))
        .map_err(|e| format!("保存配置失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_round_trip_preserves_chinese_and_disabled_rules() {
        let mut expected = defaults();
        expected[1].enabled = false;
        let json = serde_json::to_string(&Settings {
            version: 1,
            bindings: expected.iter().map(Into::into).collect(),
        })
        .unwrap();
        let actual = decode(&json).unwrap();
        for (left, right) in expected.iter().zip(actual.iter()) {
            assert_eq!(left.spec, right.spec);
            assert_eq!(left.message, right.message);
            assert_eq!(left.enabled, right.enabled);
        }
    }

    #[test]
    fn future_configuration_version_is_not_silently_replaced() {
        assert!(decode(r#"{"version":2,"bindings":[]}"#).is_err());
    }

    #[test]
    fn duplicate_recordings_and_empty_messages_are_rejected() {
        let mut values = defaults();
        let mut duplicate = values[0].clone();
        duplicate.spec.id = 42;
        values.push(duplicate);
        assert!(validate(&values).is_err());
        values.pop();
        values[0].message = " ".into();
        assert!(validate(&values).is_err());
    }
}
