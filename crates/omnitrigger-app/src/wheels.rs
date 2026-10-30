//! 自定义轮盘的动作配置；热键绑定只引用轮盘标识，槽位直接保存可执行动作。
use crate::{settings::ActionSpec, shortcut::ShortcutAction};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<WheelPosition>,
}

/// 原生方位从正上方开始顺时针编号；空方位不需要填充假动作。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WheelPosition {
    pub ring: u8,
    pub sector: u8,
    pub sectors: u8,
    #[serde(default, skip_serializing_if = "is_false")]
    pub half_step: bool,
}

/// 应用特定槽位是已经解析完成的整组替换，不在运行时解释来源的继承规则。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WheelApplicationOverride {
    pub process: String,
    pub slots: Vec<WheelSlot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WheelDefinition {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub accent: WheelAccent,
    pub slots: Vec<WheelSlot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub app_overrides: Vec<WheelApplicationOverride>,
}

impl WheelDefinition {
    pub fn validate(&self) -> Result<(), String> {
        validate_id(&self.id)?;
        validate_label(&self.name, "轮盘名称")?;
        validate_slots(&self.slots)?;
        if self.app_overrides.len() > 32 {
            return Err("每个轮盘最多配置 32 个应用覆盖。".into());
        }
        let mut processes = HashSet::new();
        for application in &self.app_overrides {
            validate_process(&application.process)?;
            if !processes.insert(application.process.to_lowercase()) {
                return Err(format!(
                    "应用覆盖重复：{}（不区分大小写）。",
                    application.process
                ));
            }
            validate_slots(&application.slots)
                .map_err(|error| format!("应用“{}”：{error}", application.process))?;
        }
        Ok(())
    }

    /// 按前台进程文件名选择完整槽位快照；未知或未提供进程时保留全局槽位。
    pub fn resolved_for_process(&self, process: Option<&str>) -> Self {
        let mut resolved = self.clone();
        if let Some(application) = process.and_then(|process| {
            let key = process.to_lowercase();
            self.app_overrides
                .iter()
                .find(|application| application.process.to_lowercase() == key)
        }) {
            resolved.slots = application.slots.clone();
        }
        resolved.app_overrides.clear();
        resolved
    }
}

fn validate_slots(slots: &[WheelSlot]) -> Result<(), String> {
    if !(1..=48).contains(&slots.len()) {
        return Err("每组轮盘需要 1–48 个槽位。".into());
    }
    let explicit = slots[0].position.is_some();
    if !explicit && slots.len() > 8 {
        return Err("未指定方位的轮盘最多包含 8 个槽位。".into());
    }
    let mut ring_counts = HashMap::new();
    let mut positions = HashSet::new();
    for (index, slot) in slots.iter().enumerate() {
        if slot.position.is_some() != explicit {
            return Err("同组轮盘槽位必须全部指定方位，或全部省略方位。".into());
        }
        if let Some(position) = slot.position {
            if position.ring > 2
                || !(1..=16).contains(&position.sectors)
                || position.sector >= position.sectors
            {
                return Err(format!(
                    "槽位 {} 方位无效：圈号应为 0–2，分区数应为 1–16，分区编号需小于分区数。",
                    index + 1
                ));
            }
            if ring_counts
                .insert(position.ring, (position.sectors, position.half_step))
                .is_some_and(|geometry| geometry != (position.sectors, position.half_step))
            {
                return Err("同一圈的所有槽位必须使用相同的分区数与半分区偏移。".into());
            }
            if !positions.insert((position.ring, position.sector)) {
                return Err(format!("槽位 {} 与已有槽位方位重复。", index + 1));
            }
        }
        validate_label(&slot.label, "槽位名称")
            .and_then(|()| match &slot.action {
                ActionSpec::Notification | ActionSpec::Window(_) => Ok(()),
                ActionSpec::Command(command) => command.validate(),
                ActionSpec::External(action) => action.validate(),
                ActionSpec::AdjustSystem(action) => action.validate(),
                ActionSpec::Local(action) => action.validate(),
                ActionSpec::Invoke(call) => call.validate().map_err(|error| error.to_string()),
                ActionSpec::Browser(action) => action.validate(),
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

fn validate_process(process: &str) -> Result<(), String> {
    if process.is_empty()
        || process.chars().count() > 260
        || process.trim() != process
        || matches!(process, "." | "..")
        || process.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*'
                )
        })
    {
        return Err("应用覆盖需要 1–260 字的进程文件名，不能包含目录、通配符或控制字符。".into());
    }
    Ok(())
}

fn is_false(value: &bool) -> bool {
    !value
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
            position: None,
        })
        .collect();
        Self {
            wheels: vec![WheelDefinition {
                id: "orbit-edit".into(),
                name: "编辑轨道".into(),
                accent: WheelAccent::Mint,
                slots,
                app_overrides: Vec::new(),
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
                let original_version: WheelsFile =
                    serde_json::from_str(&original).map_err(|error| error.to_string())?;
                if original_version.version < 4 {
                    crate::settings::backup_legacy(
                        &destination,
                        original_version.version,
                        original.as_bytes(),
                    )?;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!("读取原轮盘配置失败，原文件已保留：{error}"));
            }
        }
        let contents = serde_json::to_vec_pretty(&WheelsFile {
            version: 4,
            wheels: self.wheels.clone(),
        })
        .map_err(|error| format!("编码轮盘配置失败：{error}"))?;
        crate::settings::atomic_write(&destination, &contents)
    }

    fn decode(text: &str) -> Result<Self, String> {
        let file: WheelsFile =
            serde_json::from_str(text).map_err(|error| format!("轮盘配置格式有误：{error}"))?;
        if !(1..=4).contains(&file.version) {
            return Err("轮盘配置版本不受支持，原文件已保留。".into());
        }
        if file.version < 2
            && file.wheels.iter().any(|wheel| {
                wheel
                    .slots
                    .iter()
                    .chain(
                        wheel
                            .app_overrides
                            .iter()
                            .flat_map(|application| application.slots.iter()),
                    )
                    .any(|slot| matches!(slot.action, ActionSpec::Local(_)))
            })
        {
            return Err("本地步骤动作需要版本 2 轮盘配置。".into());
        }
        if file.version < 3
            && file.wheels.iter().any(|wheel| {
                wheel
                    .slots
                    .iter()
                    .chain(wheel.app_overrides.iter().flat_map(|app| app.slots.iter()))
                    .any(|slot| matches!(slot.action, ActionSpec::Invoke(_)))
            })
        {
            return Err("动作包调用需要版本 3 轮盘配置。".into());
        }
        if file.version < 4
            && file.wheels.iter().any(|wheel| {
                wheel
                    .slots
                    .iter()
                    .chain(wheel.app_overrides.iter().flat_map(|app| app.slots.iter()))
                    .any(|slot| matches!(slot.action, ActionSpec::Browser(_)))
            })
        {
            return Err("浏览器动作需要版本 4 轮盘配置。".into());
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
    use crate::local_action::{LocalAction, LocalStep};
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
            version: 4,
            wheels: wheels.wheels.clone(),
        })
        .unwrap()
    }

    #[test]
    fn browser_slots_preserve_profile_and_require_v4_even_in_application_override() {
        let mut wheels = Wheels::default();
        let slot = WheelSlot {
            label: "浏览器静音".into(),
            action: ActionSpec::Browser(crate::browser_actions::BrowserAction {
                command: crate::browser_actions::BrowserCommand::ToggleMute,
                profile: Some("work-browser".into()),
            }),
            position: None,
        };
        wheels.wheels[0]
            .app_overrides
            .push(WheelApplicationOverride {
                process: "chrome.exe".into(),
                slots: vec![slot],
            });
        let text = encode(&wheels);
        assert_eq!(Wheels::decode(&text).unwrap(), wheels);
        let mut old: serde_json::Value = serde_json::from_str(&text).unwrap();
        old["version"] = 3.into();
        assert_eq!(
            Wheels::decode(&old.to_string()).unwrap_err(),
            "浏览器动作需要版本 4 轮盘配置。"
        );
    }

    #[test]
    fn package_calls_round_trip_in_global_and_override_slots_and_upgrade_v2() {
        let scratch = Scratch::new();
        let mut expected = Wheels::default();
        let mut slot = expected.wheels[0].slots[0].clone();
        slot.action = ActionSpec::Invoke(omnitrigger_actions::ActionCall {
            package_id: "not.installed".into(),
            action_id: "run".into(),
            action_version: 2,
            input: serde_json::json!({"steps":["first","second"]}),
        });
        expected.wheels[0].slots[0] = slot.clone();
        expected.wheels[0].app_overrides = vec![WheelApplicationOverride {
            process: "Code.exe".into(),
            slots: vec![slot],
        }];
        assert_eq!(Wheels::decode(&encode(&expected)).unwrap(), expected);
        let mut old: serde_json::Value = serde_json::from_str(&encode(&expected)).unwrap();
        old["version"] = 2.into();
        assert!(Wheels::decode(&old.to_string())
            .unwrap_err()
            .contains("版本 3"));
        let mut original: serde_json::Value =
            serde_json::from_str(&encode(&Wheels::default())).unwrap();
        original["version"] = 2.into();
        let bytes = format!(" \r\n{original}\r\n");
        std::fs::write(scratch.wheels(), &bytes).unwrap();
        expected.save(&scratch.bindings()).unwrap();
        assert_eq!(
            std::fs::read_to_string(scratch.0.join("wheels.json.v2.bak")).unwrap(),
            bytes
        );
        assert_eq!(Wheels::load(&scratch.bindings()).unwrap(), expected);
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
                position: None,
            },
            WheelSlot {
                label: "程序".into(),
                action: ActionSpec::Command(CommandAction {
                    program: "C:\\工具\\tool.exe".into(),
                    args: vec!["中文 参数".into()],
                    working_directory: None,
                }),
                position: None,
            },
            WheelSlot {
                label: "插件".into(),
                action: ActionSpec::Plugin(PluginActionRef::raishot_capture()),
                position: None,
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
        assert_eq!(value["version"], 4);
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
        future["version"] = 5.into();
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

    fn positioned(ring: u8, sector: u8, sectors: u8, half_step: bool) -> WheelSlot {
        WheelSlot {
            label: format!("圈{ring}槽{sector}"),
            action: ActionSpec::SendShortcut(ShortcutAction { keys: vec![0x08] }),
            position: Some(WheelPosition {
                ring,
                sector,
                sectors,
                half_step,
            }),
        }
    }

    #[test]
    fn sparse_three_ring_layout_preserves_gaps_and_half_step_on_round_trip() {
        let scratch = Scratch::new();
        let mut expected = Wheels::default();
        expected.wheels[0].slots = vec![
            positioned(0, 7, 8, false),
            positioned(1, 3, 16, true),
            positioned(1, 12, 16, true),
            positioned(2, 6, 8, false),
        ];
        expected.save(&scratch.bindings()).unwrap();
        assert_eq!(Wheels::load(&scratch.bindings()).unwrap(), expected);
        let value: serde_json::Value = serde_json::from_str(&encode(&expected)).unwrap();
        assert_eq!(
            value["wheels"][0]["slots"][0]["position"],
            json!({"ring":0,"sector":7,"sectors":8})
        );
        assert_eq!(
            value["wheels"][0]["slots"][1]["position"],
            json!({"ring":1,"sector":3,"sectors":16,"half_step":true})
        );
        assert_eq!(expected.wheels[0].slots.len(), 4);
    }

    #[test]
    fn legacy_fields_stay_optional_and_legacy_and_positioned_layouts_have_separate_limits() {
        let mut wheels = Wheels::default();
        let value: serde_json::Value = serde_json::from_str(&encode(&wheels)).unwrap();
        assert!(value["wheels"][0].get("app_overrides").is_none());
        assert!(value["wheels"][0]["slots"][0].get("position").is_none());
        wheels.wheels[0].slots = vec![wheels.wheels[0].slots[0].clone(); 8];
        wheels.validate().unwrap();
        let extra = wheels.wheels[0].slots[0].clone();
        wheels.wheels[0].slots.push(extra);
        assert!(wheels.validate().is_err());
        wheels.wheels[0].slots = (0..3)
            .flat_map(|ring| (0..16).map(move |sector| positioned(ring, sector, 16, ring == 1)))
            .collect();
        assert_eq!(wheels.wheels[0].slots.len(), 48);
        wheels.validate().unwrap();
        wheels.wheels[0].slots.push(positioned(0, 0, 16, false));
        assert!(wheels.validate().is_err());
    }

    #[test]
    fn invalid_position_geometry_and_mixed_layouts_are_rejected_without_reordering() {
        for slots in [
            vec![positioned(3, 0, 8, false)],
            vec![positioned(0, 0, 0, false)],
            vec![positioned(0, 0, 17, false)],
            vec![positioned(0, 8, 8, false)],
            vec![positioned(0, 1, 8, false), positioned(0, 1, 8, false)],
            vec![positioned(0, 1, 8, false), positioned(0, 2, 16, false)],
            vec![positioned(1, 1, 16, true), positioned(1, 2, 16, false)],
            vec![
                positioned(0, 1, 8, false),
                Wheels::default().wheels[0].slots[0].clone(),
            ],
        ] {
            let mut wheels = Wheels::default();
            wheels.wheels[0].slots = slots.clone();
            assert!(wheels.validate().is_err());
            assert_eq!(wheels.wheels[0].slots, slots);
        }
    }

    #[test]
    fn application_overrides_replace_the_full_snapshot_without_mutating_the_definition() {
        let scratch = Scratch::new();
        let mut expected = Wheels::default();
        let replacement = vec![positioned(1, 7, 16, true), positioned(2, 0, 8, false)];
        expected.wheels[0].app_overrides = vec![WheelApplicationOverride {
            process: "Chrome.EXE".into(),
            slots: replacement.clone(),
        }];
        expected.save(&scratch.bindings()).unwrap();
        assert_eq!(Wheels::load(&scratch.bindings()).unwrap(), expected);
        let original = expected.wheels[0].clone();
        let resolved = original.resolved_for_process(Some("chrome.exe"));
        assert_eq!(resolved.slots, replacement);
        assert!(resolved.app_overrides.is_empty());
        assert_eq!(
            (&resolved.id, &resolved.name, resolved.accent),
            (&original.id, &original.name, original.accent)
        );
        for process in [None, Some("other.exe")] {
            let fallback = original.resolved_for_process(process);
            assert_eq!(fallback.slots, original.slots);
            assert!(fallback.app_overrides.is_empty());
        }
        assert_eq!(expected.wheels[0], original);
    }

    #[test]
    fn application_overrides_require_unique_basenames_bounded_count_and_valid_full_slots() {
        let base_slots = Wheels::default().wheels.remove(0).slots;
        for process in [
            "",
            " ",
            " chrome.exe",
            "chrome.exe ",
            "../chrome.exe",
            "C:\\chrome.exe",
            "C:chrome.exe",
            "*.exe",
            "a\nb",
            ".",
            "..",
            &"x".repeat(261),
        ] {
            let mut wheels = Wheels::default();
            wheels.wheels[0].app_overrides = vec![WheelApplicationOverride {
                process: process.into(),
                slots: base_slots.clone(),
            }];
            assert!(wheels.validate().is_err(), "错误接纳进程名 {process:?}");
        }
        let mut wheels = Wheels::default();
        wheels.wheels[0].app_overrides = vec![
            WheelApplicationOverride {
                process: "chrome.exe".into(),
                slots: base_slots.clone(),
            },
            WheelApplicationOverride {
                process: "CHROME.EXE".into(),
                slots: base_slots.clone(),
            },
        ];
        assert!(wheels.validate().is_err());
        wheels.wheels[0].app_overrides = (0..32)
            .map(|index| WheelApplicationOverride {
                process: format!("app-{index}.exe"),
                slots: base_slots.clone(),
            })
            .collect();
        wheels.validate().unwrap();
        wheels.wheels[0]
            .app_overrides
            .push(WheelApplicationOverride {
                process: "extra.exe".into(),
                slots: base_slots,
            });
        assert!(wheels.validate().is_err());
        wheels.wheels[0].app_overrides.truncate(1);
        wheels.wheels[0].app_overrides[0].slots.clear();
        assert!(wheels.validate().is_err());
        wheels.wheels[0].app_overrides[0].slots = vec![WheelSlot {
            label: "递归".into(),
            action: ActionSpec::ShowWheel(WheelActionRef {
                wheel_id: "orbit-edit".into(),
            }),
            position: None,
        }];
        assert!(wheels.validate().is_err());
    }

    #[test]
    fn extended_geometry_and_application_fields_still_reject_unknown_parameters() {
        let mut wheels = Wheels::default();
        wheels.wheels[0].slots = vec![positioned(0, 1, 8, false)];
        wheels.wheels[0].app_overrides = vec![WheelApplicationOverride {
            process: "app.exe".into(),
            slots: vec![positioned(2, 1, 8, false)],
        }];
        let base: serde_json::Value = serde_json::from_str(&encode(&wheels)).unwrap();
        for pointer in [
            "/wheels/0/slots/0/position",
            "/wheels/0/app_overrides/0",
            "/wheels/0/app_overrides/0/slots/0/position",
        ] {
            let mut value = base.clone();
            value
                .pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("future".into(), json!(true));
            assert!(Wheels::decode(&value.to_string()).is_err());
        }
    }

    fn local_slot() -> WheelSlot {
        WheelSlot {
            label: "本地步骤".into(),
            action: ActionSpec::Local(LocalAction {
                version: 1,
                steps: vec![
                    LocalStep::ReadClipboard {
                        variable: "clip".into(),
                    },
                    LocalStep::InputText {
                        text: "中文 $$\n${clip}".into(),
                    },
                    LocalStep::Delay { milliseconds: 25 },
                ],
            }),
            position: None,
        }
    }

    #[test]
    fn local_actions_round_trip_in_v2_global_and_application_slots() {
        let scratch = Scratch::new();
        let mut expected = Wheels::default();
        expected.wheels[0].slots[1] = local_slot();
        expected.wheels[0].app_overrides = vec![WheelApplicationOverride {
            process: "Code.EXE".into(),
            slots: vec![local_slot()],
        }];
        expected.save(&scratch.bindings()).unwrap();
        assert_eq!(Wheels::load(&scratch.bindings()).unwrap(), expected);
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(scratch.wheels()).unwrap()).unwrap();
        assert_eq!(saved["version"], 4);
        assert_eq!(saved["wheels"][0]["slots"][1]["action"]["kind"], "local");
        assert_eq!(
            saved["wheels"][0]["app_overrides"][0]["slots"][0]["action"]["version"],
            1
        );
        expected.wheels[0].slots.swap(0, 1);
        expected.save(&scratch.bindings()).unwrap();
        assert_eq!(Wheels::load(&scratch.bindings()).unwrap(), expected);
        assert_eq!(std::fs::read_dir(&scratch.0).unwrap().count(), 1);
    }

    #[test]
    fn local_actions_in_v1_global_or_override_slots_reject_without_overwriting() {
        let scratch = Scratch::new();
        for application_only in [false, true] {
            let mut wheels = Wheels::default();
            if application_only {
                wheels.wheels[0].app_overrides = vec![WheelApplicationOverride {
                    process: "app.exe".into(),
                    slots: vec![local_slot()],
                }];
            } else {
                wheels.wheels[0].slots[0] = local_slot();
            }
            let mut old: serde_json::Value = serde_json::from_str(&encode(&wheels)).unwrap();
            old["version"] = 1.into();
            let original = old.to_string();
            std::fs::write(scratch.wheels(), original.as_bytes()).unwrap();
            assert!(Wheels::load(&scratch.bindings())
                .unwrap_err()
                .contains("版本 2"));
            assert!(wheels.save(&scratch.bindings()).is_err());
            assert_eq!(
                std::fs::read(scratch.wheels()).unwrap(),
                original.as_bytes()
            );
        }
        assert_eq!(std::fs::read_dir(&scratch.0).unwrap().count(), 1);
    }

    #[test]
    fn adding_local_slot_to_v1_keeps_exact_legacy_backup_and_saves_v2() {
        let scratch = Scratch::new();
        let backup = scratch.0.join("wheels.json.v1.bak");
        let mut old: serde_json::Value = serde_json::from_str(&encode(&Wheels::default())).unwrap();
        old["version"] = 1.into();
        let original = format!(" \r\n{}\r\n ", old);
        std::fs::write(scratch.wheels(), original.as_bytes()).unwrap();
        let mut expected = Wheels::load(&scratch.bindings()).unwrap();
        expected.wheels[0].slots[0] = local_slot();
        expected.save(&scratch.bindings()).unwrap();
        assert_eq!(std::fs::read(&backup).unwrap(), original.as_bytes());
        assert_eq!(Wheels::load(&scratch.bindings()).unwrap(), expected);
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(scratch.wheels()).unwrap()).unwrap();
        assert_eq!(saved["version"], 4);
        expected.wheels[0].slots[0].label = "再次修改".into();
        expected.save(&scratch.bindings()).unwrap();
        assert_eq!(std::fs::read(&backup).unwrap(), original.as_bytes());
        assert_eq!(Wheels::load(&scratch.bindings()).unwrap(), expected);
        assert_eq!(std::fs::read_dir(&scratch.0).unwrap().count(), 2);
    }
}
