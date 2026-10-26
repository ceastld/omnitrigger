//! 自定义轮盘的动作配置；热键绑定只引用轮盘标识，槽位直接保存可执行动作。
use crate::{settings::ActionSpec, shortcut::ShortcutAction};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::Path};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WheelActionRef {
    pub wheel_id: String,
}

impl WheelActionRef {
    /// 只校验引用格式；删除或缺失的轮盘由界面标记为不可用，不改写绑定。
    pub fn validate(&self) -> Result<(), String> {
        validate_id(&self.wheel_id)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WheelAccent {
    #[default]
    Mint,
    Violet,
    Amber,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WheelSlot {
    pub label: String,
    pub action: ActionSpec,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WheelDefinition {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub accent: WheelAccent,
    pub slots: Vec<WheelSlot>,
}

impl WheelDefinition {
    pub fn validate(&self) -> Result<(), String> {
        validate_id(&self.id)?;
        validate_label(&self.name, "轮盘名称")?;
        if !(1..=8).contains(&self.slots.len()) {
            return Err("每个轮盘需要 1–8 个槽位。".into());
        }
        for (index, slot) in self.slots.iter().enumerate() {
            validate_label(&slot.label, "槽位名称")
                .and_then(|()| match &slot.action {
                    ActionSpec::Notification => Ok(()),
                    ActionSpec::Command(command) => command.validate(),
                    ActionSpec::SendShortcut(shortcut) => shortcut.validate(),
                    ActionSpec::Plugin(reference) => {
                        reference.validate().map_err(|error| error.to_string())
                    }
                    ActionSpec::ShowPanel | ActionSpec::ShowWheel(_) => {
                        Err("轮盘槽位不能打开面板或其他轮盘。".into())
                    }
                })
                .map_err(|error| format!("槽位 {}：{error}", index + 1))?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wheels {
    pub wheels: Vec<WheelDefinition>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WheelsFile {
    version: u32,
    wheels: Vec<WheelDefinition>,
}

impl Default for Wheels {
    fn default() -> Self {
        let slots = [
            ("复制", 0x43),
            ("粘贴", 0x56),
            ("全选", 0x41),
            ("撤销", 0x5a),
        ]
        .into_iter()
        .map(|(label, terminal)| WheelSlot {
            label: label.into(),
            action: ActionSpec::SendShortcut(ShortcutAction {
                keys: vec![0xa2, terminal],
            }),
        })
        .collect();
        Self {
            wheels: vec![WheelDefinition {
                id: "orbit-edit".into(),
                name: "编辑轨道".into(),
                accent: WheelAccent::Mint,
                slots,
            }],
        }
    }
}

impl Wheels {
    /// 配置路径为当前 bindings.json；轮盘存于同目录 wheels.json。
    pub fn load(config_path: &Path) -> Result<Self, String> {
        let destination = config_path.with_file_name("wheels.json");
        match std::fs::read_to_string(destination) {
            Ok(text) => Self::decode(&text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(format!("读取轮盘配置失败：{error}")),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.wheels.len() > 16 {
            return Err("最多可配置 16 个轮盘。".into());
        }
        let mut ids = HashSet::new();
        for wheel in &self.wheels {
            wheel
                .validate()
                .map_err(|error| format!("轮盘“{}”：{error}", wheel.name))?;
            if !ids.insert(&wheel.id) {
                return Err(format!("轮盘标识重复：{}。", wheel.id));
            }
        }
        Ok(())
    }

    /// 原文件无法读取或校验时拒绝覆盖；写完整临时文件后再原子替换。
    pub fn save(&self, config_path: &Path) -> Result<(), String> {
        self.validate()?;
        let destination = config_path.with_file_name("wheels.json");
        match std::fs::read_to_string(&destination) {
            Ok(original) => {
                Self::decode(&original)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!("读取原轮盘配置失败，原文件已保留：{error}"));
            }
        }
        let contents = serde_json::to_vec_pretty(&WheelsFile {
            version: 1,
            wheels: self.wheels.clone(),
        })
        .map_err(|error| format!("编码轮盘配置失败：{error}"))?;
        crate::settings::atomic_write(&destination, &contents)
    }

    fn decode(text: &str) -> Result<Self, String> {
        let file: WheelsFile =
            serde_json::from_str(text).map_err(|error| format!("轮盘配置格式有误：{error}"))?;
        if file.version != 1 {
            return Err("轮盘配置版本不受支持，原文件已保留。".into());
        }
        let wheels = Self {
            wheels: file.wheels,
        };
        wheels.validate()?;
        Ok(wheels)
    }
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, b'-' | b'_'))
    {
        return Err("轮盘标识需要 1–64 个 ASCII 字母、数字、连字符或下划线。".into());
    }
    Ok(())
}

fn validate_label(label: &str, field: &str) -> Result<(), String> {
    if label.trim().is_empty() || label.chars().count() > 30 || label.chars().any(char::is_control)
    {
        return Err(format!(
            "{field}需要 1–30 个字符，不能全为空白或包含控制字符。"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{execution::CommandAction, plugins::PluginActionRef};
    use serde_json::json;
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let directory = std::env::temp_dir().join(format!(
                "omnitrigger-wheels-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&directory).unwrap();
            Self(directory)
        }

        fn bindings(&self) -> PathBuf {
            self.0.join("bindings.json")
        }

        fn wheels(&self) -> PathBuf {
            self.0.join("wheels.json")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn encode(wheels: &Wheels) -> String {
        serde_json::to_string(&WheelsFile {
            version: 1,
            wheels: wheels.wheels.clone(),
        })
        .unwrap()
    }

    #[test]
    fn absent_file_has_editing_slots_without_registering_or_writing_bindings() {
        let scratch = Scratch::new();
        let wheels = Wheels::load(&scratch.bindings()).unwrap();
        assert_eq!(wheels, Wheels::default());
        wheels.validate().unwrap();
        let editing = &wheels.wheels[0];
        assert_eq!(editing.id, "orbit-edit");
        assert_eq!(editing.name, "编辑轨道");
        assert_eq!(editing.accent, WheelAccent::Mint);
        for (slot, terminal) in editing.slots.iter().zip([0x43, 0x56, 0x41, 0x5a]) {
            assert_eq!(
                slot.action,
                ActionSpec::SendShortcut(ShortcutAction {
                    keys: vec![0xa2, terminal]
                })
            );
        }
        assert!(!scratch.bindings().exists());
        assert!(!scratch.wheels().exists());
    }

    #[test]
    fn round_trip_preserves_order_accents_and_independent_slot_actions() {
        let scratch = Scratch::new();
        std::fs::write(scratch.bindings(), "绑定原样保留").unwrap();
        let mut expected = Wheels::default();
        let mut second = expected.wheels[0].clone();
        second.id = "custom_2".into();
        second.name = "自定义".into();
        second.accent = WheelAccent::Violet;
        second.slots = vec![
            WheelSlot {
                label: "提示".into(),
                action: ActionSpec::Notification,
            },
            WheelSlot {
                label: "程序".into(),
                action: ActionSpec::Command(CommandAction {
                    program: "C:\\工具\\tool.exe".into(),
                    args: vec!["中文 参数".into()],
                    working_directory: None,
                }),
            },
            WheelSlot {
                label: "插件".into(),
                action: ActionSpec::Plugin(PluginActionRef::raishot_capture()),
            },
        ];
        expected.wheels.push(second);
        expected.save(&scratch.bindings()).unwrap();
        assert_eq!(Wheels::load(&scratch.bindings()).unwrap(), expected);
        expected.wheels[0].accent = WheelAccent::Amber;
        expected.wheels.swap(0, 1);
        expected.save(&scratch.bindings()).unwrap();
        assert_eq!(Wheels::load(&scratch.bindings()).unwrap(), expected);
        assert_eq!(
            std::fs::read_to_string(scratch.bindings()).unwrap(),
            "绑定原样保留"
        );
        let value: serde_json::Value = serde_json::from_str(&encode(&expected)).unwrap();
        assert_eq!(value["version"], 1);
        assert_eq!(value["wheels"][1]["accent"], "amber");
    }

    #[test]
    fn invalid_ids_duplicates_labels_and_sizes_are_rejected() {
        for id in ["", "中文", "a/b", "has space", ".", &"a".repeat(65)] {
            assert!(WheelActionRef {
                wheel_id: id.into()
            }
            .validate()
            .is_err());
        }
        assert!(WheelActionRef {
            wheel_id: "A-z_09".into()
        }
        .validate()
        .is_ok());
        let valid = Wheels::default();
        let mut invalid = valid.clone();
        invalid.wheels.push(valid.wheels[0].clone());
        assert!(invalid.validate().is_err());
        invalid.wheels = (0..17)
            .map(|index| WheelDefinition {
                id: format!("wheel-{index}"),
                ..valid.wheels[0].clone()
            })
            .collect();
        assert!(invalid.validate().is_err());
        for label in ["", "  ", "a\nb", &"中".repeat(31)] {
            let mut wheel = valid.wheels[0].clone();
            wheel.name = label.into();
            assert!(wheel.validate().is_err());
            wheel.name = "轮盘".into();
            wheel.slots[0].label = label.into();
            assert!(wheel.validate().is_err());
        }
        for count in [0, 9] {
            let mut wheel = valid.wheels[0].clone();
            wheel.slots = vec![wheel.slots[0].clone(); count];
            assert!(wheel.validate().is_err());
        }
    }

    #[test]
    fn rejects_recursive_slots_and_invalid_actions_without_rewriting_them() {
        for action in [
            ActionSpec::ShowPanel,
            ActionSpec::ShowWheel(WheelActionRef {
                wheel_id: "orbit-edit".into(),
            }),
            ActionSpec::SendShortcut(ShortcutAction { keys: vec![] }),
            ActionSpec::Command(CommandAction {
                program: String::new(),
                args: vec![],
                working_directory: None,
            }),
        ] {
            let mut wheels = Wheels::default();
            wheels.wheels[0].slots[0].action = action.clone();
            assert!(wheels.validate().is_err());
            assert_eq!(wheels.wheels[0].slots[0].action, action);
        }
    }

    #[test]
    fn unknown_fields_future_versions_and_bad_originals_are_never_overwritten() {
        let scratch = Scratch::new();
        let wheels = Wheels::default();
        let base: serde_json::Value = serde_json::from_str(&encode(&wheels)).unwrap();
        let mut bad_values = vec!["not-json".to_string()];
        let mut future = base.clone();
        future["version"] = 2.into();
        bad_values.push(future.to_string());
        for pointer in [
            "",
            "/wheels/0",
            "/wheels/0/slots/0",
            "/wheels/0/slots/0/action",
        ] {
            let mut value = base.clone();
            value
                .pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("unknown".into(), json!(true));
            bad_values.push(value.to_string());
        }
        let mut recursive = base.clone();
        recursive["wheels"][0]["slots"][0]["action"] =
            json!({"kind":"show_wheel", "wheel_id":"orbit-edit"});
        bad_values.push(recursive.to_string());
        for original in bad_values {
            std::fs::write(scratch.wheels(), &original).unwrap();
            assert!(Wheels::load(&scratch.bindings()).is_err());
            assert!(wheels.save(&scratch.bindings()).is_err());
            assert_eq!(std::fs::read_to_string(scratch.wheels()).unwrap(), original);
        }
    }

    #[test]
    fn absent_accent_defaults_to_mint_and_explicit_empty_collection_is_preserved() {
        let wheels = Wheels::default();
        let mut value: serde_json::Value = serde_json::from_str(&encode(&wheels)).unwrap();
        value["wheels"][0].as_object_mut().unwrap().remove("accent");
        assert_eq!(Wheels::decode(&value.to_string()).unwrap(), wheels);
        assert!(Wheels::decode(r#"{"version":1,"wheels":[]}"#)
            .unwrap()
            .wheels
            .is_empty());
    }

    #[test]
    fn invalid_edits_leave_the_last_valid_file_unchanged() {
        let scratch = Scratch::new();
        let mut wheels = Wheels::default();
        wheels.save(&scratch.bindings()).unwrap();
        let original = std::fs::read(scratch.wheels()).unwrap();
        wheels.wheels[0].slots.clear();
        assert!(wheels.save(&scratch.bindings()).is_err());
        assert_eq!(std::fs::read(scratch.wheels()).unwrap(), original);
        assert_eq!(
            Wheels::load(&scratch.bindings()).unwrap(),
            Wheels::default()
        );
    }
}
