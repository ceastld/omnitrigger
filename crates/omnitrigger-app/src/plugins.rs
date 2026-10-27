//! 声明式插件目录、本机设置与调用解析。所有文件访问均在输入 Hook 之外。
use crate::execution::CommandAction;
use omnitrigger_core::{Activation, Button, Engine, MouseButton, TriggerSpec, WheelDirection};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt, path::Path};

const RAISHOT_MANIFEST: &str = include_str!("../plugins/raishot.json");

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginActionRef {
    pub plugin_id: String,
    pub action_id: String,
    pub action_version: u32,
    #[serde(default)]
    pub parameters: BTreeMap<String, serde_json::Value>,
}

impl PluginActionRef {
    pub fn raishot_capture() -> Self {
        Self {
            plugin_id: "raishot".into(),
            action_id: "capture".into(),
            action_version: 1,
            parameters: BTreeMap::new(),
        }
    }

    /// 只检查引用结构，不要求目标已安装；丢失插件的绑定仍可保存和修复。
    pub fn validate(&self) -> Result<(), PluginError> {
        if !valid_name(&self.plugin_id) || !valid_name(&self.action_id) || self.action_version == 0
        {
            return Err(PluginError::InvalidReference);
        }
        if self.parameters.len() > 128
            || serde_json::to_vec(&self.parameters)
                .map_err(|error| PluginError::InvalidParameters(error.to_string()))?
                .len()
                > 64 * 1024
        {
            return Err(PluginError::InvalidParameters(
                "动作参数不能超过 128 项或 64 KiB。".into(),
            ));
        }
        Ok(())
    }
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'-' | b'_' | b'.'))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginError {
    InvalidReference,
    PluginMissing(String),
    Disabled,
    PathNotConfigured,
    InvalidPath(String),
    ProgramMissing(String),
    ActionMissing(String),
    IncompatibleVersion,
    InvalidParameters(String),
    InvalidManifest(String),
    Settings(String),
}

impl PluginError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidReference => "invalid_reference",
            Self::PluginMissing(_) => "plugin_missing",
            Self::Disabled => "plugin_disabled",
            Self::PathNotConfigured => "path_not_configured",
            Self::InvalidPath(_) => "invalid_path",
            Self::ProgramMissing(_) => "program_missing",
            Self::ActionMissing(_) => "action_missing",
            Self::IncompatibleVersion => "incompatible_version",
            Self::InvalidParameters(_) => "invalid_parameters",
            Self::InvalidManifest(_) => "invalid_manifest",
            Self::Settings(_) => "plugin_settings",
        }
    }
}

impl fmt::Display for PluginError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidReference => formatter.write_str("插件动作引用格式有误。"),
            Self::PluginMissing(id) => write!(formatter, "插件 {id} 不可用，请安装对应适配。"),
            Self::Disabled => formatter.write_str("RaiShot 插件已停用，请在动作库中启用。"),
            Self::PathNotConfigured => {
                formatter.write_str("尚未配置 RaiShot CLI，请在动作库中安装或选择程序。")
            }
            Self::InvalidPath(reason) => write!(formatter, "RaiShot CLI 路径无效：{reason}"),
            Self::ProgramMissing(path) => {
                write!(formatter, "找不到 RaiShot CLI：{path}，请重新定位程序。")
            }
            Self::ActionMissing(id) => {
                write!(formatter, "RaiShot 不提供动作 {id}，请检查适配版本。")
            }
            Self::IncompatibleVersion => formatter.write_str("插件动作版本不兼容，原绑定已保留。"),
            Self::InvalidParameters(reason) => write!(formatter, "插件动作参数有误：{reason}"),
            Self::InvalidManifest(reason) => write!(formatter, "插件清单有误：{reason}"),
            Self::Settings(reason) => write!(formatter, "插件设置错误：{reason}"),
        }
    }
}

impl std::error::Error for PluginError {}

#[derive(Clone, Debug)]
pub struct ResolvedPluginAction {
    pub command: CommandAction,
    pub group: String,
    pub plugin_version: String,
}

#[derive(Clone)]
pub struct Plugins {
    settings: PluginSettings,
    manifest: Result<PluginManifest, PluginError>,
    available: Result<(), PluginError>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginSettings {
    version: u32,
    raishot: RaiShotSettings,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RaiShotSettings {
    enabled: bool,
    cli_path: String,
}

impl Default for Plugins {
    fn default() -> Self {
        let mut plugins = Self {
            settings: PluginSettings {
                version: 1,
                raishot: RaiShotSettings {
                    enabled: true,
                    cli_path: String::new(),
                },
            },
            manifest: PluginManifest::decode(RAISHOT_MANIFEST),
            available: Err(PluginError::PathNotConfigured),
        };
        plugins.refresh();
        plugins
    }
}

impl Plugins {
    pub fn load(config_path: &Path) -> Result<Self, PluginError> {
        let destination = settings_directory(config_path).join("plugins.json");
        let text = match std::fs::read_to_string(&destination) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => return Err(PluginError::Settings(format!("读取失败：{error}"))),
        };
        let settings = decode_settings(&text)?;
        let mut plugins = Self {
            settings,
            ..Self::default()
        };
        plugins.refresh();
        Ok(plugins)
    }

    pub fn save(&self, config_path: &Path) -> Result<(), PluginError> {
        let destination = settings_directory(config_path).join("plugins.json");
        match std::fs::read_to_string(&destination) {
            Ok(text) => {
                decode_settings(&text)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(PluginError::Settings(format!("读取原设置失败：{error}"))),
        }
        let text = serde_json::to_vec_pretty(&self.settings)
            .map_err(|error| PluginError::Settings(error.to_string()))?;
        crate::settings::atomic_write(&destination, &text).map_err(PluginError::Settings)
    }

    pub fn raishot_path(&self) -> &str {
        &self.settings.raishot.cli_path
    }

    pub fn raishot_enabled(&self) -> bool {
        self.settings.raishot.enabled
    }

    pub fn configure_raishot(&mut self, path: String, enabled: bool) -> Result<(), PluginError> {
        if path.contains('\0') || path.len() > 64 * 1024 {
            return Err(PluginError::InvalidPath("路径过长或含空字符。".into()));
        }
        if enabled {
            validate_program(&path)?;
        }
        let mut candidate = self.clone();
        candidate.settings.raishot = RaiShotSettings {
            enabled,
            cli_path: path,
        };
        candidate.refresh();
        candidate.manifest.as_ref().map_err(Clone::clone)?;
        *self = candidate;
        Ok(())
    }

    /// 在设置刷新等应用层时机更新缓存，渲染和实际注册仅查询缓存。
    pub fn refresh(&mut self) {
        self.available = self.manifest.as_ref().map_err(Clone::clone).and_then(|_| {
            if !self.settings.raishot.enabled {
                return Err(PluginError::Disabled);
            }
            validate_program(&self.settings.raishot.cli_path)
        });
    }

    pub fn availability(&self, reference: &PluginActionRef) -> Result<(), PluginError> {
        self.definition(reference)?;
        self.available.clone()
    }

    pub fn concurrency_group(&self, reference: &PluginActionRef) -> Option<String> {
        let action = self.definition(reference).ok()?;
        Some(format!(
            "{}/{}",
            reference.plugin_id, action.concurrency.group
        ))
    }

    pub fn default_trigger(
        &self,
        reference: &PluginActionRef,
        id: u64,
    ) -> Result<TriggerSpec, PluginError> {
        let spec = self.definition(reference)?.default_trigger.spec(id);
        Engine::new(vec![spec.clone()])
            .map_err(|error| PluginError::InvalidManifest(error.to_string()))?;
        Ok(spec)
    }

    pub fn resolve(
        &self,
        reference: &PluginActionRef,
    ) -> Result<ResolvedPluginAction, PluginError> {
        let action = self.definition(reference)?;
        if !self.settings.raishot.enabled {
            return Err(PluginError::Disabled);
        }
        validate_program(&self.settings.raishot.cli_path)?;
        let program = self.settings.raishot.cli_path.clone();
        let command = CommandAction {
            working_directory: Path::new(&program)
                .parent()
                .map(|path| path.to_string_lossy().into_owned()),
            program,
            args: action
                .runner
                .args
                .iter()
                .map(|argument| argument.literal.clone())
                .collect(),
        };
        command.validate().map_err(PluginError::InvalidParameters)?;
        let manifest = self.manifest.as_ref().map_err(Clone::clone)?;
        Ok(ResolvedPluginAction {
            command,
            group: format!("{}/{}", manifest.id, action.concurrency.group),
            plugin_version: manifest.version.clone(),
        })
    }

    fn definition(&self, reference: &PluginActionRef) -> Result<&ActionDefinition, PluginError> {
        reference.validate()?;
        let manifest = self.manifest.as_ref().map_err(Clone::clone)?;
        if reference.plugin_id != manifest.id {
            return Err(PluginError::PluginMissing(reference.plugin_id.clone()));
        }
        let action = manifest
            .actions
            .iter()
            .find(|action| action.id == reference.action_id)
            .ok_or_else(|| PluginError::ActionMissing(reference.action_id.clone()))?;
        if reference.action_version != action.version {
            return Err(PluginError::IncompatibleVersion);
        }
        if !reference.parameters.is_empty() {
            return Err(PluginError::InvalidParameters(
                "截图动作不接受参数。".into(),
            ));
        }
        Ok(action)
    }
}

fn settings_directory(config_path: &Path) -> &Path {
    config_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn decode_settings(text: &str) -> Result<PluginSettings, PluginError> {
    let settings: PluginSettings = serde_json::from_str(text)
        .map_err(|error| PluginError::Settings(format!("格式有误，原文件已保留：{error}")))?;
    if settings.version != 1 {
        return Err(PluginError::Settings(
            "不支持此设置版本，原文件已保留。".into(),
        ));
    }
    if settings.raishot.cli_path.len() > 64 * 1024 || settings.raishot.cli_path.contains('\0') {
        return Err(PluginError::Settings("程序路径过长或含空字符。".into()));
    }
    Ok(settings)
}

fn validate_program(path: &str) -> Result<(), PluginError> {
    if path.trim().is_empty() {
        return Err(PluginError::PathNotConfigured);
    }
    if path.contains('\0') || path.len() > 64 * 1024 {
        return Err(PluginError::InvalidPath("路径过长或含空字符。".into()));
    }
    let program = Path::new(path);
    if !program.is_absolute() {
        return Err(PluginError::InvalidPath(
            "请使用程序的完整路径，不加外层引号。".into(),
        ));
    }
    let metadata = std::fs::metadata(program).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            PluginError::ProgramMissing(path.into())
        } else {
            PluginError::InvalidPath(error.to_string())
        }
    })?;
    if !metadata.is_file() {
        return Err(PluginError::InvalidPath("所选路径不是文件。".into()));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // 不对云端占位程序触发下载；插件安装和刷新都不执行程序。
        if metadata.file_attributes() & (0x1000 | 0x40000 | 0x400000) != 0 {
            return Err(PluginError::InvalidPath(
                "程序仍是云端占位文件，请先将其保存在本机。".into(),
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PluginManifest {
    schema_version: u32,
    host_api_version: u32,
    id: String,
    version: String,
    name: String,
    description: String,
    settings: Vec<SettingDefinition>,
    actions: Vec<ActionDefinition>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingDefinition {
    key: String,
    #[serde(rename = "type")]
    kind: String,
    title: String,
    required: bool,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ActionDefinition {
    id: String,
    version: u32,
    title: String,
    description: String,
    parameters: Vec<serde_json::Value>,
    default_trigger: DefaultTrigger,
    runner: ProcessRunner,
    concurrency: Concurrency,
    effects: Vec<String>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProcessRunner {
    kind: String,
    program: SettingReference,
    args: Vec<LiteralArgument>,
    working_directory: ParentReference,
    completion: String,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingReference {
    setting: String,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct LiteralArgument {
    literal: String,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ParentReference {
    parent_of_setting: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Concurrency {
    group: String,
    max_running: u32,
    on_busy: String,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct DefaultTrigger {
    held: Vec<ButtonDescription>,
    activation: ActivationDescription,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum ButtonDescription {
    Key(u16),
    Left,
    Right,
    Middle,
    X1,
    X2,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum ActivationDescription {
    Press(ButtonDescription),
    Hold {
        button: ButtonDescription,
        duration_ms: u32,
    },
    WheelUp,
    WheelDown,
    WheelLeft,
    WheelRight,
}

impl ButtonDescription {
    fn button(self) -> Button {
        match self {
            Self::Key(code) => Button::Key(code),
            Self::Left => Button::Mouse(MouseButton::Left),
            Self::Right => Button::Mouse(MouseButton::Right),
            Self::Middle => Button::Mouse(MouseButton::Middle),
            Self::X1 => Button::Mouse(MouseButton::X1),
            Self::X2 => Button::Mouse(MouseButton::X2),
        }
    }
}

impl DefaultTrigger {
    fn spec(&self, id: u64) -> TriggerSpec {
        TriggerSpec {
            conditions: Vec::new(),
            id,
            held: self.held.iter().map(|button| button.button()).collect(),
            activation: match self.activation {
                ActivationDescription::Press(button) => Activation::Press(button.button()),
                ActivationDescription::Hold {
                    button,
                    duration_ms,
                } => Activation::Hold {
                    button: button.button(),
                    duration_ms,
                },
                ActivationDescription::WheelUp => Activation::Wheel(WheelDirection::Up),
                ActivationDescription::WheelDown => Activation::Wheel(WheelDirection::Down),
                ActivationDescription::WheelLeft => Activation::Wheel(WheelDirection::Left),
                ActivationDescription::WheelRight => Activation::Wheel(WheelDirection::Right),
            },
        }
    }
}

impl PluginManifest {
    fn decode(text: &str) -> Result<Self, PluginError> {
        if text.len() > 64 * 1024 {
            return Err(PluginError::InvalidManifest("清单超过 64 KiB。".into()));
        }
        let manifest: Self = serde_json::from_str(text)
            .map_err(|error| PluginError::InvalidManifest(error.to_string()))?;
        if manifest.schema_version != 1 || manifest.host_api_version != 1 {
            return Err(PluginError::IncompatibleVersion);
        }
        if manifest.id != "raishot"
            || !valid_name(&manifest.version)
            || manifest.name.trim().is_empty()
            || manifest.description.trim().is_empty()
            || manifest.settings.len() != 1
            || manifest.actions.len() != 1
        {
            return Err(PluginError::InvalidManifest(
                "首版只支持内置 RaiShot 适配。".into(),
            ));
        }
        let setting = &manifest.settings[0];
        if setting.key != "cliPath"
            || setting.kind != "executable_path"
            || !setting.required
            || setting.title.trim().is_empty()
        {
            return Err(PluginError::InvalidManifest("缺少 CLI 路径设置。".into()));
        }
        let action = &manifest.actions[0];
        if action.id != "capture"
            || action.version != 1
            || action.title.trim().is_empty()
            || action.description.trim().is_empty()
            || !action.parameters.is_empty()
            || action.effects != ["screen_capture"]
            || action.runner.kind != "process"
            || action.runner.program.setting != "cliPath"
            || action.runner.working_directory.parent_of_setting != "cliPath"
            || action.runner.completion != "process_exit"
            || action.runner.args.len() != 1
            || action.runner.args[0].literal != "--capture"
            || action.concurrency.group != "capture"
            || action.concurrency.max_running != 1
            || action.concurrency.on_busy != "reject"
        {
            return Err(PluginError::InvalidManifest(
                "截图调用或并发契约不受支持。".into(),
            ));
        }
        Engine::new(vec![action.default_trigger.spec(1)])
            .map_err(|error| PluginError::InvalidManifest(error.to_string()))?;
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    fn scratch() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "omnitrigger-plugins-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn builtin_manifest_and_default_trigger_are_valid_without_installing_program() {
        let plugins = Plugins::default();
        let reference = PluginActionRef::raishot_capture();
        assert!(matches!(
            plugins.availability(&reference),
            Err(PluginError::PathNotConfigured)
        ));
        let trigger = plugins.default_trigger(&reference, 42).unwrap();
        assert_eq!(trigger.id, 42);
        assert_eq!(trigger.held, vec![Button::Key(0xa2), Button::Key(0xa4)]);
        assert_eq!(trigger.activation, Activation::Press(Button::Key(0x41)));
        assert!(PluginManifest::decode(
            &RAISHOT_MANIFEST.replace("\"maxRunning\": 1", "\"maxRunning\": 2")
        )
        .is_err());
        assert!(PluginManifest::decode(
            &RAISHOT_MANIFEST.replace("\"hostApiVersion\": 1", "\"hostApiVersion\": 2")
        )
        .is_err());
    }

    #[test]
    fn resolves_saved_unicode_path_and_refreshes_missing_program_without_losing_settings() {
        let directory = scratch();
        std::fs::create_dir_all(&directory).unwrap();
        let executable = directory.join("中文 截图 & tool.exe");
        std::fs::write(&executable, "仅测试路径，不执行").unwrap();
        let config_path = directory.join("custom-bindings.json");
        let reference = PluginActionRef::raishot_capture();
        let mut plugins = Plugins::default();
        plugins
            .configure_raishot(executable.to_string_lossy().into_owned(), true)
            .unwrap();
        plugins.save(&config_path).unwrap();
        let mut loaded = Plugins::load(&config_path).unwrap();
        assert!(loaded.availability(&reference).is_ok());
        let resolved = loaded.resolve(&reference).unwrap();
        assert_eq!(resolved.command.program, executable.to_string_lossy());
        assert_eq!(resolved.command.args, ["--capture"]);
        assert_eq!(
            resolved.command.working_directory.as_deref(),
            Some(directory.to_str().unwrap())
        );
        assert_eq!(resolved.group, "raishot/capture");
        std::fs::remove_file(&executable).unwrap();
        assert!(matches!(
            loaded.resolve(&reference),
            Err(PluginError::ProgramMissing(_))
        ));
        loaded.refresh();
        assert!(matches!(
            loaded.availability(&reference),
            Err(PluginError::ProgramMissing(_))
        ));
        assert_eq!(loaded.raishot_path(), executable.to_str().unwrap());
        std::fs::remove_file(directory.join("plugins.json")).unwrap();
        std::fs::remove_dir(&directory).unwrap();
    }

    #[test]
    fn rejects_unknown_identity_version_and_parameters_independently_of_install_state() {
        let plugins = Plugins::default();
        let mut reference = PluginActionRef::raishot_capture();
        reference.plugin_id = "missing".into();
        assert!(matches!(
            plugins.availability(&reference),
            Err(PluginError::PluginMissing(_))
        ));
        reference.plugin_id = "raishot".into();
        reference.action_id = "missing".into();
        assert!(matches!(
            plugins.availability(&reference),
            Err(PluginError::ActionMissing(_))
        ));
        reference.action_id = "capture".into();
        reference.action_version = 2;
        assert!(matches!(
            plugins.availability(&reference),
            Err(PluginError::IncompatibleVersion)
        ));
        reference.action_version = 1;
        reference.parameters.insert("unknown".into(), true.into());
        assert!(matches!(
            plugins.availability(&reference),
            Err(PluginError::InvalidParameters(_))
        ));
        reference.plugin_id = "../raishot".into();
        assert!(matches!(
            reference.validate(),
            Err(PluginError::InvalidReference)
        ));
    }

    #[test]
    fn invalid_configuration_is_not_overwritten_and_disable_keeps_path() {
        let directory = scratch();
        std::fs::create_dir_all(&directory).unwrap();
        let config_path = directory.join("bindings.json");
        let destination = directory.join("plugins.json");
        let original = "{\"version\":99,\"raishot\":{\"enabled\":true,\"cliPath\":\"\"}}";
        std::fs::write(&destination, original).unwrap();
        assert!(Plugins::load(&config_path).is_err());
        assert!(Plugins::default().save(&config_path).is_err());
        assert_eq!(std::fs::read_to_string(&destination).unwrap(), original);
        let mut plugins = Plugins::default();
        plugins.configure_raishot("旧路径".into(), false).unwrap();
        assert!(!plugins.raishot_enabled());
        assert_eq!(plugins.raishot_path(), "旧路径");
        assert_eq!(
            plugins
                .availability(&PluginActionRef::raishot_capture())
                .unwrap_err()
                .code(),
            "plugin_disabled"
        );
        assert!(plugins
            .configure_raishot("relative.exe".into(), true)
            .is_err());
        assert_eq!(plugins.raishot_path(), "旧路径");
        std::fs::remove_file(destination).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
