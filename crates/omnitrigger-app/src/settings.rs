//! 应用持久化边界。纯核心无需了解文件格式或用户目录。
use omnitrigger_core::{
    Activation, Button, Engine, KeyboardMatching, KeyboardPolicy, MouseButton, ProcessScope,
    RegistrationError, TriggerSpec, WheelDirection, WheelPolicy,
};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    OnceLock,
};

use crate::execution::CommandAction;
use crate::external_action::ExternalAction;
use crate::local_action::LocalAction;
use crate::plugins::PluginActionRef;
use crate::shortcut::ShortcutAction;
use crate::system_action::SystemAdjustmentAction;
use crate::trigger_conditions::ConditionDto;
use crate::wheels::WheelActionRef;
use crate::window_action::WindowAction;

static CUSTOM_PATH: OnceLock<PathBuf> = OnceLock::new();
const CONFIG_VERSION: u32 = 18;

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
    Window(WindowAction),
    External(ExternalAction),
    AdjustSystem(SystemAdjustmentAction),
    Local(LocalAction),
    Plugin(PluginActionRef),
    Invoke(omnitrigger_actions::ActionCall),
    Browser(crate::browser_actions::BrowserAction),
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
    #[serde(default, skip_serializing_if = "ProcessScopeDto::is_global")]
    pointer_scope: ProcessScopeDto,
    #[serde(default, skip_serializing_if = "WheelPolicyDto::is_default")]
    wheel: WheelPolicyDto,
    #[serde(default, skip_serializing_if = "KeyboardPolicyDto::is_default")]
    keyboard: KeyboardPolicyDto,
    #[serde(default, skip_serializing_if = "ProcessScopeDto::is_global")]
    process_scope: ProcessScopeDto,
    #[serde(rename = "id")]
    id: u64,
    #[serde(rename = "held")]
    held: Vec<ButtonDto>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    conditions: Vec<ConditionDto>,
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

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WheelPolicyDto {
    #[default]
    Notch,
    Packet,
}

impl WheelPolicyDto {
    fn is_default(&self) -> bool {
        matches!(self, Self::Notch)
    }
}

impl From<WheelPolicyDto> for WheelPolicy {
    fn from(policy: WheelPolicyDto) -> Self {
        match policy {
            WheelPolicyDto::Notch => Self::Notch,
            WheelPolicyDto::Packet => Self::Packet,
        }
    }
}

impl From<WheelPolicy> for WheelPolicyDto {
    fn from(policy: WheelPolicy) -> Self {
        match policy {
            WheelPolicy::Notch => Self::Notch,
            WheelPolicy::Packet => Self::Packet,
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyboardPolicyDto {
    #[serde(default, skip_serializing_if = "KeyboardMatchingDto::is_default")]
    matching: KeyboardMatchingDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dedicated_prefix: Option<ButtonDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    retained_prefix: Option<RetainedPrefixDto>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    repeat: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    restore_on_tap: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tap_timeout_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "ProcessScopeDto::is_global")]
    tap_scope: ProcessScopeDto,
}

impl KeyboardPolicyDto {
    fn is_default(&self) -> bool {
        self.matching.is_default()
            && self.dedicated_prefix.is_none()
            && self.retained_prefix.is_none()
            && !self.repeat
            && !self.restore_on_tap
            && self.tap_timeout_ms.is_none()
            && self.tap_scope.is_global()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetainedPrefixDto {
    key: ButtonDto,
    #[serde(default, skip_serializing_if = "ProcessScopeDto::is_global")]
    capture_scope: ProcessScopeDto,
    quiet_ms: u32,
    fast_ms: u32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    block_with_primary_mouse: bool,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum KeyboardMatchingDto {
    #[default]
    Exact,
    MouseExact,
    Captured {
        priority: u16,
    },
}
impl KeyboardMatchingDto {
    fn is_default(&self) -> bool {
        matches!(self, Self::Exact)
    }
}
impl From<KeyboardMatching> for KeyboardMatchingDto {
    fn from(value: KeyboardMatching) -> Self {
        match value {
            KeyboardMatching::Exact => Self::Exact,
            KeyboardMatching::MouseExact => Self::MouseExact,
            KeyboardMatching::Captured { priority } => Self::Captured { priority },
        }
    }
}
impl From<KeyboardMatchingDto> for KeyboardMatching {
    fn from(value: KeyboardMatchingDto) -> Self {
        match value {
            KeyboardMatchingDto::Exact => Self::Exact,
            KeyboardMatchingDto::MouseExact => Self::MouseExact,
            KeyboardMatchingDto::Captured { priority } => Self::Captured { priority },
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessScopeDto {
    #[serde(default, skip_serializing_if = "ProcessIdentityDto::is_default")]
    identity: ProcessIdentityDto,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    allow: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    deny: Vec<String>,
}

impl ProcessScopeDto {
    fn is_global(&self) -> bool {
        self.allow.is_empty() && self.deny.is_empty() && self.identity.is_default()
    }
}

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProcessIdentityDto {
    #[default]
    Executable,
    ShellContext,
}
impl ProcessIdentityDto {
    fn is_default(&self) -> bool {
        matches!(self, Self::Executable)
    }
}
impl From<omnitrigger_core::ProcessIdentity> for ProcessIdentityDto {
    fn from(value: omnitrigger_core::ProcessIdentity) -> Self {
        match value {
            omnitrigger_core::ProcessIdentity::Executable => Self::Executable,
            omnitrigger_core::ProcessIdentity::ShellContext => Self::ShellContext,
        }
    }
}
impl From<ProcessIdentityDto> for omnitrigger_core::ProcessIdentity {
    fn from(value: ProcessIdentityDto) -> Self {
        match value {
            ProcessIdentityDto::Executable => Self::Executable,
            ProcessIdentityDto::ShellContext => Self::ShellContext,
        }
    }
}

pub fn process_scope_label(scope: &ProcessScope) -> String {
    let mut text = if scope.allow.is_empty() {
        "所有应用".into()
    } else {
        format!("仅 {}", scope.allow.join("、"))
    };
    if !scope.deny.is_empty() {
        text += &format!("，排除 {}", scope.deny.join("、"));
    }
    if scope.identity == omnitrigger_core::ProcessIdentity::ShellContext {
        format!("窗口上下文：{text}")
    } else {
        text
    }
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
    Hold {
        button: ButtonDto,
        duration_ms: u32,
    },
    Drag {
        button: ButtonDto,
        min_distance_px: u32,
        max_start_ms: u32,
    },
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
            wheel: binding.spec.wheel.into(),
            pointer_scope: ProcessScopeDto {
                identity: binding.spec.pointer_scope.identity.into(),
                allow: binding.spec.pointer_scope.allow.clone(),
                deny: binding.spec.pointer_scope.deny.clone(),
            },
            keyboard: KeyboardPolicyDto {
                matching: binding.spec.keyboard.matching.into(),
                dedicated_prefix: binding.spec.keyboard.dedicated_prefix.map(Into::into),
                retained_prefix: binding
                    .spec
                    .keyboard
                    .retained_prefix
                    .as_ref()
                    .map(|policy| RetainedPrefixDto {
                        key: policy.key.into(),
                        capture_scope: ProcessScopeDto {
                            identity: policy.capture_scope.identity.into(),
                            allow: policy.capture_scope.allow.clone(),
                            deny: policy.capture_scope.deny.clone(),
                        },
                        quiet_ms: policy.quiet_ms,
                        fast_ms: policy.fast_ms,
                        block_with_primary_mouse: policy.block_with_primary_mouse,
                    }),
                repeat: binding.spec.keyboard.repeat,
                restore_on_tap: binding.spec.keyboard.restore_on_tap,
                tap_timeout_ms: binding.spec.keyboard.tap_timeout_ms,
                tap_scope: ProcessScopeDto {
                    identity: binding.spec.keyboard.tap_scope.identity.into(),
                    allow: binding.spec.keyboard.tap_scope.allow.clone(),
                    deny: binding.spec.keyboard.tap_scope.deny.clone(),
                },
            },
            process_scope: ProcessScopeDto {
                identity: binding.spec.process_scope.identity.into(),
                allow: binding.spec.process_scope.allow.clone(),
                deny: binding.spec.process_scope.deny.clone(),
            },
            id: binding.spec.id,
            held: binding.spec.held.iter().copied().map(Into::into).collect(),
            conditions: binding.spec.conditions.iter().map(Into::into).collect(),
            activation: match binding.spec.activation {
                Activation::Press(button) => ActivationDto::Press(button.into()),
                Activation::Hold {
                    button,
                    duration_ms,
                } => ActivationDto::Hold {
                    button: button.into(),
                    duration_ms,
                },
                Activation::Drag {
                    button,
                    min_distance_px,
                    max_start_ms,
                } => ActivationDto::Drag {
                    button: button.into(),
                    min_distance_px,
                    max_start_ms,
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
        if version < 18 && matches!(self.keyboard.matching, KeyboardMatchingDto::MouseExact) {
            return Err("鼠标修饰键观察匹配需要版本 18 配置。".into());
        }
        if version < 17 && !self.pointer_scope.is_global() {
            return Err("指针窗口范围需要版本 17 配置。".into());
        }
        if version < 16 && !self.wheel.is_default() {
            return Err("逐事件滚轮策略需要版本 16 配置。".into());
        }
        if version < 15
            && (!self.process_scope.identity.is_default()
                || !self.keyboard.tap_scope.identity.is_default()
                || self
                    .keyboard
                    .retained_prefix
                    .as_ref()
                    .is_some_and(|policy| !policy.capture_scope.identity.is_default()))
        {
            return Err("窗口上下文匹配需要版本 15 配置。".into());
        }
        if version < 14 && self.keyboard.retained_prefix.is_some() {
            return Err("保留原功能前缀需要版本 14 配置。".into());
        }
        if version < 12 && !self.keyboard.matching.is_default() {
            return Err("捕获集合匹配需要版本 12 配置。".into());
        }
        if version < 11 && !self.keyboard.is_default() {
            return Err("专用前缀与键盘重复需要版本 11 配置。".into());
        }
        if version < 10 && !self.process_scope.is_global() {
            return Err("应用范围需要版本 10 配置。".into());
        }
        if version < 6 && !self.conditions.is_empty() {
            return Err("位置条件需要版本 6 配置。".into());
        }
        if version < 8
            && self.conditions.iter().any(|condition| {
                matches!(
                    condition,
                    ConditionDto::PointerRegion {
                        region: crate::trigger_conditions::RegionDto::ScreenEdgeSegment { .. }
                    }
                )
            })
        {
            return Err("边缘区段需要版本 8 配置。".into());
        }
        let action = if version >= 5 {
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
        if version < 6 && matches!(action, ActionSpec::AdjustSystem(_)) {
            return Err("系统调节动作需要版本 6 配置。".into());
        }
        if version < 7 && matches!(action, ActionSpec::Local(_)) {
            return Err("本地步骤动作需要版本 7 配置。".into());
        }
        if version < 8 && matches!(action, ActionSpec::Invoke(_)) {
            return Err("动作包调用需要版本 8 配置。".into());
        }
        if version < 9 && matches!(action, ActionSpec::Browser(_)) {
            return Err("浏览器动作需要版本 9 配置。".into());
        }
        if version < 13
            && matches!(&action, ActionSpec::SendShortcut(shortcut) if !shortcut.timing.is_default())
        {
            return Err("按键时序需要版本 13 配置。".into());
        }
        Ok(Binding {
            spec: TriggerSpec {
                pointer_scope: ProcessScope {
                    identity: self.pointer_scope.identity.into(),
                    allow: self.pointer_scope.allow,
                    deny: self.pointer_scope.deny,
                },
                wheel: self.wheel.into(),
                keyboard: KeyboardPolicy {
                    matching: self.keyboard.matching.into(),
                    dedicated_prefix: self.keyboard.dedicated_prefix.map(Into::into),
                    retained_prefix: self.keyboard.retained_prefix.map(|policy| {
                        Box::new(omnitrigger_core::RetainedPrefix {
                            key: policy.key.into(),
                            capture_scope: ProcessScope {
                                identity: policy.capture_scope.identity.into(),
                                allow: policy.capture_scope.allow,
                                deny: policy.capture_scope.deny,
                            },
                            quiet_ms: policy.quiet_ms,
                            fast_ms: policy.fast_ms,
                            block_with_primary_mouse: policy.block_with_primary_mouse,
                        })
                    }),
                    repeat: self.keyboard.repeat,
                    restore_on_tap: self.keyboard.restore_on_tap,
                    tap_timeout_ms: self.keyboard.tap_timeout_ms,
                    tap_scope: ProcessScope {
                        identity: self.keyboard.tap_scope.identity.into(),
                        allow: self.keyboard.tap_scope.allow,
                        deny: self.keyboard.tap_scope.deny,
                    },
                },
                process_scope: ProcessScope {
                    identity: self.process_scope.identity.into(),
                    allow: self.process_scope.allow,
                    deny: self.process_scope.deny,
                },
                conditions: self.conditions.into_iter().map(Into::into).collect(),
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
                    ActivationDto::Drag {
                        button,
                        min_distance_px,
                        max_start_ms,
                    } => Activation::Drag {
                        button: button.into(),
                        min_distance_px,
                        max_start_ms,
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
                pointer_scope: Default::default(),
                wheel: Default::default(),
                keyboard: Default::default(),
                process_scope: Default::default(),
                conditions: Vec::new(),
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
                pointer_scope: Default::default(),
                wheel: Default::default(),
                keyboard: Default::default(),
                process_scope: Default::default(),
                conditions: Vec::new(),
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
                pointer_scope: Default::default(),
                wheel: Default::default(),
                keyboard: Default::default(),
                process_scope: Default::default(),
                conditions: Vec::new(),
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
                pointer_scope: Default::default(),
                wheel: Default::default(),
                keyboard: Default::default(),
                process_scope: Default::default(),
                conditions: Vec::new(),
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
                pointer_scope: Default::default(),
                wheel: Default::default(),
                keyboard: Default::default(),
                process_scope: Default::default(),
                conditions: Vec::new(),
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
    if !matches!(settings.version, 1..=CONFIG_VERSION) {
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
        && spec.conditions.is_empty()
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
        RegistrationError::ConflictingModifierDelivery { first, second } => Conflict {
            title: "修饰键交付策略冲突",
            detail: "同一修饰键在重叠范围内同时用于鼠标观察和输入捕获，请调整范围或统一交付策略。".into(),
            other: pick(first, second),
        },
        RegistrationError::InvalidWheelPolicy { .. } => Conflict {
            title: "滚轮策略不适用于当前触发方式",
            detail: "逐事件策略仅用于滚轮，请改为滚轮触发或恢复缺省策略。".into(),
            other: None,
        },
        RegistrationError::CapturedPriorityConflict { first, second } => Conflict {
            title: "捕获规则顺序冲突",
            detail: "同一捕获层的重叠规则使用了相同顺序编号，请在配置中明确 matching.priority。".into(),
            other: pick(first, second),
        },
        RegistrationError::DuplicateTrigger { first, second } => Conflict {
            title: "与已有触发范围冲突",
            detail: "组合和位置范围存在重叠；可调整区域、滚轮方向或按住组合。普通按下不能与重叠范围的同组合长按或快拖并用。".into(),
            other: pick(first, second),
        },
        RegistrationError::InvalidDragButton { .. } => Conflict {
            title: "拖拽目标必须是鼠标按钮",
            detail: "快速拖拽只支持鼠标按钮，键盘按键可选择按下或长按。".into(),
            other: None,
        },
        RegistrationError::InvalidDragDistance { min_distance_px, .. } => Conflict {
            title: "拖拽距离无效",
            detail: format!("拖拽距离应为 4–500 个物理像素，当前为 {min_distance_px}px。"),
            other: None,
        },
        RegistrationError::InvalidDragWindow { max_start_ms, .. } => Conflict {
            title: "快拖时窗无效",
            detail: format!("快速拖拽时窗应为 20–2000ms，当前为 {max_start_ms}ms。"),
            other: None,
        },
        RegistrationError::TimingConflict {
            hold_binding,
            drag_binding,
            hold_duration_ms,
            drag_max_start_ms,
        } => Conflict {
            title: "长按与快拖时序冲突",
            detail: format!("快速拖拽时窗（{drag_max_start_ms}ms）必须严格短于同组合的长按阈值（{hold_duration_ms}ms）。"),
            other: pick(hold_binding, drag_binding),
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
            ActionSpec::External(action) => action.validate(),
            ActionSpec::AdjustSystem(action) => action.validate(),
            ActionSpec::Local(action) => action.validate(),
            ActionSpec::Invoke(call) => call.validate().map_err(|error| error.to_string()),
            ActionSpec::Browser(action) => action.validate(),
            ActionSpec::ShowWheel(reference) => reference.validate(),
            ActionSpec::Plugin(reference) => {
                reference.validate().map_err(|error| error.to_string())
            }
            ActionSpec::Notification | ActionSpec::ShowPanel | ActionSpec::Window(_) => Ok(()),
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
        version: CONFIG_VERSION,
        bindings: bindings.iter().map(Into::into).collect(),
    })
    .map_err(|e| e.to_string())?;
    match std::fs::read_to_string(destination) {
        Ok(original) => {
            // 不覆盖损坏或未来版本的配置；用户需要先明确修复原文件。
            decode(&original)?;
            let settings: Settings =
                serde_json::from_str(&original).map_err(|error| error.to_string())?;
            if settings.version < CONFIG_VERSION {
                backup_legacy(destination, settings.version, original.as_bytes())?;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("读取原配置失败，原文件已保留：{error}")),
    }
    atomic_write(destination, &text)
}

pub(crate) fn backup_legacy(
    destination: &Path,
    version: u32,
    original: &[u8],
) -> Result<(), String> {
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
    use crate::local_action::LocalStep;
    use crate::system_action::AdjustmentTarget;
    use omnitrigger_core::{PointerRegion, ScreenCorner, ScreenEdge, ScreenRect, TriggerCondition};
    use serde_json::json;

    fn encode(bindings: &[Binding]) -> String {
        serde_json::to_string(&Settings {
            version: CONFIG_VERSION,
            bindings: bindings.iter().map(Into::into).collect(),
        })
        .unwrap()
    }

    #[test]
    fn keyboard_policy_round_trip_upgrade_and_mislabeled_schema_preserve_original_files() {
        let scratch = Scratch::new();
        let destination = scratch.config();
        let mut document: serde_json::Value = serde_json::from_str(&encode(&defaults())).unwrap();
        document["version"] = 10.into();
        let original = format!(" \r\n{document}\r\n");
        std::fs::write(&destination, &original).unwrap();
        let mut bindings = defaults();
        for binding in &mut bindings {
            if binding.spec.held.contains(&Button::Key(20)) {
                binding.spec.keyboard = KeyboardPolicy {
                    dedicated_prefix: Some(Button::Key(20)),
                    repeat: true,
                    ..Default::default()
                };
            }
        }
        save_to(&destination, &bindings).unwrap();
        assert_eq!(
            std::fs::read_to_string(scratch.0.join("bindings.json.v10.bak")).unwrap(),
            original
        );
        let saved = std::fs::read_to_string(&destination).unwrap();
        assert_eq!(decode(&saved).unwrap(), bindings);
        let mut document: serde_json::Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(
            document["bindings"][0]["keyboard"],
            json!({"dedicated_prefix":{"kind":"key","value":20},"repeat":true})
        );
        document["version"] = 10.into();
        assert!(decode(&document.to_string())
            .unwrap_err()
            .contains("版本 11"));
        document["version"] = CONFIG_VERSION.into();
        document["bindings"][0]["keyboard"]["unexpected"] = true.into();
        assert!(decode(&document.to_string()).is_err());
        bindings[0].spec.keyboard = Default::default();
        assert!(save_to(&destination, &bindings)
            .unwrap_err()
            .contains("专用层"));
        assert_eq!(std::fs::read_to_string(&destination).unwrap(), saved);
    }

    #[test]
    fn packet_wheel_round_trip_requires_v16_and_keeps_exact_v15_backup() {
        let scratch = Scratch::new();
        let destination = scratch.config();
        let mut document: serde_json::Value = serde_json::from_str(&encode(&defaults())).unwrap();
        document["version"] = 15.into();
        let original = format!(" \r\n{document}\r\n");
        std::fs::write(&destination, &original).unwrap();
        let mut bindings = decode(&original).unwrap();
        assert!(bindings
            .iter()
            .all(|binding| binding.spec.wheel == WheelPolicy::Notch));
        bindings[0].spec.activation = Activation::Wheel(WheelDirection::Up);
        bindings[0].spec.wheel = WheelPolicy::Packet;
        save_to(&destination, &bindings).unwrap();
        assert_eq!(
            std::fs::read_to_string(scratch.0.join("bindings.json.v15.bak")).unwrap(),
            original
        );
        let saved = std::fs::read_to_string(&destination).unwrap();
        assert_eq!(decode(&saved).unwrap(), bindings);
        let mut document: serde_json::Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(document["version"], CONFIG_VERSION);
        assert_eq!(document["bindings"][0]["wheel"], "packet");
        assert!(document["bindings"][1].get("wheel").is_none());
        document["version"] = 15.into();
        assert!(decode(&document.to_string())
            .unwrap_err()
            .contains("版本 16"));
        document["version"] = 16.into();
        document["bindings"][0]["wheel"] = "unknown".into();
        assert!(decode(&document.to_string()).is_err());
        bindings[0].spec.activation = Activation::Press(Button::Key(87));
        assert!(save_to(&destination, &bindings)
            .unwrap_err()
            .contains("仅支持滚轮"));
        assert_eq!(std::fs::read_to_string(destination).unwrap(), saved);
    }

    #[test]
    fn pointer_scope_round_trip_requires_v17_and_preserves_exact_v16_backup() {
        let scratch = Scratch::new();
        let destination = scratch.config();
        let mut doc: serde_json::Value = serde_json::from_str(&encode(&defaults())).unwrap();
        doc["version"] = 16.into();
        let original = format!(" \r\n{doc}\r\n");
        std::fs::write(&destination, &original).unwrap();
        let mut bindings = decode(&original).unwrap();
        bindings[0].spec.pointer_scope = ProcessScope {
            identity: omnitrigger_core::ProcessIdentity::ShellContext,
            allow: vec!["desktop".into()],
            deny: vec![],
        };
        save_to(&destination, &bindings).unwrap();
        assert_eq!(
            std::fs::read_to_string(scratch.0.join("bindings.json.v16.bak")).unwrap(),
            original
        );
        let saved = std::fs::read_to_string(&destination).unwrap();
        assert_eq!(decode(&saved).unwrap(), bindings);
        let mut doc: serde_json::Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(doc["version"], CONFIG_VERSION);
        assert_eq!(
            doc["bindings"][0]["pointer_scope"],
            json!({"identity":"shell_context","allow":["desktop"]})
        );
        assert!(doc["bindings"][1].get("pointer_scope").is_none());
        doc["version"] = 16.into();
        assert!(decode(&doc.to_string()).unwrap_err().contains("版本 17"));
        doc["version"] = 17.into();
        doc["bindings"][0]["pointer_scope"]["subject"] = "unknown".into();
        assert!(decode(&doc.to_string()).is_err());
        bindings[0].spec.pointer_scope.allow = vec!["bad/path".into()];
        assert!(save_to(&destination, &bindings)
            .unwrap_err()
            .contains("指针窗口范围"));
        assert_eq!(std::fs::read_to_string(destination).unwrap(), saved);
    }

    #[test]
    fn mouse_exact_round_trip_requires_v18_and_keeps_generic_modifier() {
        let mut doc = json!({"version":18,"bindings":[{
            "id":1,"held":[{"kind":"key","value":17}],
            "activation":{"kind":"wheel_up"},"wheel":"packet",
            "keyboard":{"matching":{"kind":"mouse_exact"}},
            "message":"测试","enabled":true,"action":{"kind":"notification"}
        }]});
        let bindings = decode(&doc.to_string()).unwrap();
        assert_eq!(
            bindings[0].spec.keyboard.matching,
            KeyboardMatching::MouseExact
        );
        assert_eq!(bindings[0].spec.held, vec![Button::Key(17)]);
        let saved = encode(&bindings);
        assert_eq!(decode(&saved).unwrap(), bindings);
        let saved_doc: serde_json::Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(
            saved_doc["bindings"][0]["keyboard"]["matching"]["kind"],
            "mouse_exact"
        );
        assert_eq!(saved_doc["bindings"][0]["held"][0]["value"], 17);
        doc["version"] = 17.into();
        assert!(decode(&doc.to_string()).unwrap_err().contains("版本 18"));
        doc["version"] = 18.into();
        doc["bindings"][0]["held"][0]["value"] = 32.into();
        assert!(decode(&doc.to_string())
            .unwrap_err()
            .contains("鼠标精确观察模式"));
    }

    #[test]
    fn side_tab_example_matches_packets_clicks_and_keeps_release_and_tap_fallback() {
        use omnitrigger_core::{Axis, InputEvent, InputKind, MouseButton};
        let bindings = decode(include_str!("../../../examples/mouse-tab-navigation.json")).unwrap();
        assert_eq!(bindings.len(), 3);
        assert!(bindings.iter().all(|binding| !binding.enabled));
        let mut engine =
            Engine::new(bindings.into_iter().map(|binding| binding.spec).collect()).unwrap();
        engine.set_foreground_process(Some("chrome")).unwrap();
        engine
            .set_pointer_context(Some("chrome"), omnitrigger_core::WindowRole::Application)
            .unwrap();
        let input = |engine: &mut Engine, kind| {
            engine.process(InputEvent {
                kind,
                injected: false,
            })
        };
        let side = Button::Mouse(MouseButton::X1);
        let right = Button::Mouse(MouseButton::Right);
        assert!(input(&mut engine, InputKind::Down(side)).consume);
        for (delta, id) in [(15, 1), (-240, 2)] {
            let result = input(
                &mut engine,
                InputKind::Wheel {
                    axis: Axis::Vertical,
                    delta,
                },
            );
            assert!(result.consume);
            assert_eq!(result.triggered, [id]);
        }
        assert_eq!(input(&mut engine, InputKind::Down(right)).triggered, [3]);
        for key in [right, side] {
            let result = input(&mut engine, InputKind::Up(key));
            assert!(result.consume && result.replay.is_empty());
        }
        input(&mut engine, InputKind::Down(side));
        assert_eq!(
            input(&mut engine, InputKind::Up(side)).replay,
            [InputKind::Down(side), InputKind::Up(side)]
        );
        let ctrl = Button::Key(162);
        input(&mut engine, InputKind::Down(ctrl));
        assert!(!input(&mut engine, InputKind::Down(side)).consume);
        assert!(
            !input(
                &mut engine,
                InputKind::Wheel {
                    axis: Axis::Vertical,
                    delta: 120
                }
            )
            .consume
        );
        input(&mut engine, InputKind::Up(side));
        input(&mut engine, InputKind::Up(ctrl));
        engine
            .set_pointer_context(None, omnitrigger_core::WindowRole::Unknown)
            .unwrap();
        assert!(!input(&mut engine, InputKind::Down(side)).consume);
    }

    #[test]
    fn repeat_on_hold_is_rejected_without_dropping_the_recorded_keys() {
        let mut bindings = vec![defaults()[0].clone()];
        bindings[0].spec.keyboard.repeat = true;
        bindings[0].spec.activation = Activation::Hold {
            button: Button::Key(87),
            duration_ms: 300,
        };
        assert!(decode(&encode(&bindings))
            .unwrap_err()
            .contains("重复仅支持"));
        assert_eq!(bindings[0].spec.held, vec![Button::Key(20)]);
    }

    #[test]
    fn tap_restore_policy_keeps_timeout_and_rejects_impossible_combinations() {
        let mut bindings = vec![defaults()[0].clone()];
        bindings[0].spec.keyboard = KeyboardPolicy {
            matching: Default::default(),
            dedicated_prefix: Some(Button::Key(20)),
            retained_prefix: None,
            restore_on_tap: true,
            tap_timeout_ms: Some(20_000),
            repeat: true,
            tap_scope: ProcessScope {
                identity: Default::default(),
                allow: vec!["Wormhole.exe".into()],
                deny: vec![],
            },
        };
        assert_eq!(decode(&encode(&bindings)).unwrap(), bindings);
        let document: serde_json::Value = serde_json::from_str(&encode(&bindings)).unwrap();
        assert_eq!(
            document["bindings"][0]["keyboard"]["tap_timeout_ms"],
            20_000
        );
        bindings[0].spec.keyboard.restore_on_tap = false;
        assert!(decode(&encode(&bindings)).unwrap_err().contains("单按恢复"));
        bindings[0].spec.keyboard.restore_on_tap = true;
        bindings[0].spec.keyboard.dedicated_prefix = None;
        assert!(decode(&encode(&bindings)).unwrap_err().contains("单按恢复"));
    }

    #[test]
    fn captured_matching_round_trip_requires_v12_and_keeps_the_exact_v11_backup() {
        let scratch = Scratch::new();
        let destination = scratch.config();
        let mut document: serde_json::Value = serde_json::from_str(&encode(&defaults())).unwrap();
        document["version"] = 11.into();
        let original = format!(" \r\n{document}\r\n");
        std::fs::write(&destination, &original).unwrap();
        let mut bindings = vec![defaults()[0].clone()];
        bindings[0].spec.keyboard = KeyboardPolicy {
            dedicated_prefix: Some(Button::Key(20)),
            matching: KeyboardMatching::Captured { priority: 7 },
            repeat: true,
            ..Default::default()
        };
        save_to(&destination, &bindings).unwrap();
        assert_eq!(
            std::fs::read_to_string(scratch.0.join("bindings.json.v11.bak")).unwrap(),
            original
        );
        let conflict = check_candidate(&bindings, &bindings[0].spec).unwrap_err();
        assert_eq!(conflict.other, Some(bindings[0].spec.id));
        assert_eq!(conflict.title, "捕获规则顺序冲突");
        assert_eq!(
            decode(&std::fs::read_to_string(&destination).unwrap()).unwrap(),
            bindings
        );
        let mut saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&destination).unwrap()).unwrap();
        assert_eq!(saved["version"], CONFIG_VERSION);
        assert_eq!(
            saved["bindings"][0]["keyboard"]["matching"],
            json!({"kind":"captured","priority":7})
        );
        saved["version"] = 11.into();
        assert!(decode(&saved.to_string()).unwrap_err().contains("版本 12"));
        saved["version"] = 12.into();
        saved["bindings"][0]["keyboard"]["matching"]["priority"] = 512.into();
        assert!(decode(&saved.to_string()).unwrap_err().contains("捕获集合"));
        assert_eq!(
            decode(&std::fs::read_to_string(&destination).unwrap()).unwrap(),
            bindings
        );
    }

    #[test]
    fn retained_prefix_requires_v14_preserves_v13_backup_and_round_trips() {
        let scratch = Scratch::new();
        let destination = scratch.config();
        let mut bindings = vec![defaults()[0].clone()];
        let key = bindings[0].spec.held[0];
        let mut original: serde_json::Value = serde_json::from_str(&encode(&bindings)).unwrap();
        original["version"] = 13.into();
        let original = format!(" \r\n{original}\r\n");
        std::fs::write(&destination, &original).unwrap();
        bindings[0].spec.keyboard = KeyboardPolicy {
            matching: KeyboardMatching::Captured { priority: 3 },
            retained_prefix: Some(Box::new(omnitrigger_core::RetainedPrefix {
                key,
                capture_scope: Default::default(),
                quiet_ms: 50,
                fast_ms: 50,
                block_with_primary_mouse: true,
            })),
            repeat: true,
            restore_on_tap: true,
            tap_timeout_ms: Some(1500),
            ..Default::default()
        };
        assert_eq!(decode(&encode(&bindings)).unwrap(), bindings);
        save_to(&destination, &bindings).unwrap();
        assert_eq!(
            std::fs::read(scratch.0.join("bindings.json.v13.bak")).unwrap(),
            original.as_bytes()
        );
        let mut value: serde_json::Value = serde_json::from_str(&encode(&bindings)).unwrap();
        value["version"] = 13.into();
        assert!(decode(&value.to_string()).unwrap_err().contains("版本 14"));
        value["version"] = 14.into();
        value["bindings"][0]["keyboard"]["retained_prefix"]["unknown"] = true.into();
        assert!(decode(&value.to_string()).is_err());
        assert_eq!(
            std::fs::read(scratch.0.join("bindings.json.v13.bak")).unwrap(),
            original.as_bytes()
        );
    }

    #[test]
    fn shell_context_requires_v15_preserves_v14_and_all_three_scope_domains() {
        let scratch = Scratch::new();
        let destination = scratch.config();
        let mut old: serde_json::Value = serde_json::from_str(&encode(&defaults())).unwrap();
        old["version"] = 14.into();
        let original = format!(" \r\n{old}\r\n");
        std::fs::write(&destination, original.as_bytes()).unwrap();
        let mut binding = defaults()[0].clone();
        binding.spec.held = vec![Button::Key(32)];
        binding.spec.activation = Activation::Press(Button::Key(75));
        binding.spec.process_scope = ProcessScope {
            identity: omnitrigger_core::ProcessIdentity::ShellContext,
            allow: vec!["desktop".into()],
            deny: vec![],
        };
        binding.spec.keyboard = KeyboardPolicy {
            matching: KeyboardMatching::Captured { priority: 0 },
            retained_prefix: Some(Box::new(omnitrigger_core::RetainedPrefix {
                key: Button::Key(32),
                capture_scope: binding.spec.process_scope.clone(),
                quiet_ms: 50,
                fast_ms: 50,
                block_with_primary_mouse: false,
            })),
            restore_on_tap: true,
            tap_timeout_ms: Some(1500),
            tap_scope: binding.spec.process_scope.clone(),
            ..Default::default()
        };
        let bindings = vec![binding];
        assert_eq!(decode(&encode(&bindings)).unwrap(), bindings);
        save_to(&destination, &bindings).unwrap();
        assert_eq!(
            std::fs::read(scratch.0.join("bindings.json.v14.bak")).unwrap(),
            original.as_bytes()
        );
        let base: serde_json::Value = serde_json::from_str(&encode(&bindings)).unwrap();
        for path in [
            "/bindings/0/process_scope",
            "/bindings/0/keyboard/tap_scope",
            "/bindings/0/keyboard/retained_prefix/capture_scope",
        ] {
            let mut document = base.clone();
            for other in [
                "/bindings/0/process_scope",
                "/bindings/0/keyboard/tap_scope",
                "/bindings/0/keyboard/retained_prefix/capture_scope",
            ] {
                if other != path {
                    document
                        .pointer_mut(other)
                        .unwrap()
                        .as_object_mut()
                        .unwrap()
                        .remove("identity");
                }
            }
            document["version"] = 14.into();
            assert!(decode(&document.to_string())
                .unwrap_err()
                .contains("版本 15"));
            document["version"] = 15.into();
            document.pointer_mut(path).unwrap()["identity"] = "future".into();
            assert!(decode(&document.to_string()).is_err());
        }
        assert!(process_scope_label(&bindings[0].spec.process_scope).starts_with("窗口上下文："));
    }

    #[test]
    fn shortcut_timing_requires_v13_and_keeps_exact_v12_backup() {
        let scratch = Scratch::new();
        let destination = scratch.config();
        let mut document: serde_json::Value = serde_json::from_str(&encode(&defaults())).unwrap();
        document["version"] = 12.into();
        let original = format!(" \r\n{document}\r\n");
        std::fs::write(&destination, &original).unwrap();
        let mut bindings = vec![defaults()[0].clone()];
        let mut shortcut = ShortcutAction::new(vec![163, 165, 161, 65]);
        shortcut
            .timing
            .process_overrides
            .insert("WPS.exe".into(), 10);
        bindings[0].action = ActionSpec::SendShortcut(shortcut);
        save_to(&destination, &bindings).unwrap();
        assert_eq!(
            std::fs::read_to_string(scratch.0.join("bindings.json.v12.bak")).unwrap(),
            original
        );
        assert_eq!(
            decode(&std::fs::read_to_string(&destination).unwrap()).unwrap(),
            bindings
        );
        let mut saved: serde_json::Value = serde_json::from_str(&encode(&bindings)).unwrap();
        saved["version"] = 12.into();
        assert!(decode(&saved.to_string()).unwrap_err().contains("版本 13"));
        assert_eq!(
            std::fs::read_to_string(scratch.0.join("bindings.json.v12.bak")).unwrap(),
            original
        );
    }

    #[derive(Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
    enum ReplayEvent {
        Down {
            button: ButtonDto,
        },
        Up {
            button: ButtonDto,
        },
        Wheel {
            axis: ReplayAxis,
            delta: i32,
        },
        Advance {},
        Foreground {
            process: Option<String>,
            role: Option<String>,
        },
        Pointer {
            process: Option<String>,
            role: Option<String>,
        },
        Cancel {},
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum ReplayAxis {
        Vertical,
        Horizontal,
    }

    impl ReplayEvent {
        fn input(self) -> Option<omnitrigger_core::InputKind> {
            use omnitrigger_core::{Axis, InputKind};
            Some(match self {
                Self::Down { button } => InputKind::Down(button.into()),
                Self::Up { button } => InputKind::Up(button.into()),
                Self::Wheel { axis, delta } => InputKind::Wheel {
                    axis: match axis {
                        ReplayAxis::Vertical => Axis::Vertical,
                        ReplayAxis::Horizontal => Axis::Horizontal,
                    },
                    delta,
                },
                _ => return None,
            })
        }
    }

    fn replay_event(value: &serde_json::Value) -> ReplayEvent {
        if value.get("kind").is_some() {
            serde_json::from_value(value.clone()).unwrap()
        } else {
            let button = ButtonDto::Key(u16::try_from(value["key"].as_u64().unwrap()).unwrap());
            if value["down"].as_bool().unwrap() {
                ReplayEvent::Down { button }
            } else {
                ReplayEvent::Up { button }
            }
        }
    }

    fn replay_role(role: Option<&str>) -> omnitrigger_core::WindowRole {
        use omnitrigger_core::WindowRole;
        match role {
            None | Some("unknown") => WindowRole::Unknown,
            Some("application") => WindowRole::Application,
            Some("desktop") => WindowRole::Desktop,
            Some("taskbar") => WindowRole::Taskbar,
            Some(other) => panic!("未知窗口角色：{other}"),
        }
    }

    fn replay_document(document: &serde_json::Value) -> (usize, usize) {
        let bindings = decode(&document["config"].to_string()).unwrap();
        assert!(bindings.iter().all(|binding| !binding.enabled));
        let specs: Vec<_> = bindings.into_iter().map(|binding| binding.spec).collect();
        let mut engine = Engine::new(specs.clone()).unwrap();
        let cases = document["cases"].as_array().unwrap();
        assert!(!cases.is_empty() && cases.len() <= 100_000);
        for (case_index, case) in cases.iter().enumerate() {
            // 带绝对时间的独立见证需重建引擎，避免上一组的单调时钟污染边界。
            if case
                .get("reset")
                .is_some_and(|value| value.as_bool().unwrap())
            {
                engine = Engine::new(specs.clone()).unwrap();
            }
            engine
                .set_foreground_context(
                    case["process"].as_str(),
                    replay_role(case["role"].as_str()),
                )
                .unwrap();
            if case.get("pointer_process").is_some() {
                engine
                    .set_pointer_context(
                        case["pointer_process"].as_str(),
                        replay_role(case["pointer_role"].as_str()),
                    )
                    .unwrap();
            }
            let steps = case["steps"].as_array().unwrap();
            assert!(!steps.is_empty() && steps.len() <= 32);
            for (step_index, step) in steps.iter().enumerate() {
                let event = replay_event(step.get("event").unwrap_or(step));
                let time = step.get("time_ms").map(|value| value.as_u64().unwrap());
                let decision = match event {
                    ReplayEvent::Advance {} => {
                        engine.advance_time(time.expect("推进时间需要 time_ms"))
                    }
                    ReplayEvent::Foreground { process, role } => {
                        assert!(time.is_none(), "上下文变更前请单独推进时间");
                        engine
                            .set_foreground_context(
                                process.as_deref(),
                                replay_role(role.as_deref()),
                            )
                            .unwrap()
                    }
                    ReplayEvent::Pointer { process, role } => {
                        assert!(time.is_none(), "上下文变更前请单独推进时间");
                        engine
                            .set_pointer_context(process.as_deref(), replay_role(role.as_deref()))
                            .unwrap()
                    }
                    ReplayEvent::Cancel {} => {
                        assert!(time.is_none(), "取消前请单独推进时间");
                        engine.cancel()
                    }
                    event => {
                        let event = omnitrigger_core::InputEvent {
                            kind: event.input().unwrap(),
                            injected: step
                                .get("injected")
                                .is_some_and(|value| value.as_bool().unwrap()),
                        };
                        if let Some(time) = time {
                            engine.process_at(event, time)
                        } else {
                            engine.process(event)
                        }
                    }
                };
                let expected: Vec<u64> = serde_json::from_value(step["triggered"].clone()).unwrap();
                assert_eq!(
                    decision.triggered, expected,
                    "case={case_index} step={step_index}"
                );
                let expected_replay = step["replay"]
                    .as_array()
                    .map(|rows| {
                        rows.iter()
                            .map(|row| replay_event(row).input().expect("补发只能包含输入事件"))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                assert_eq!(
                    decision.replay, expected_replay,
                    "case={case_index} step={step_index}"
                );
                if let Some(consume) = step["consume"].as_bool() {
                    assert_eq!(
                        decision.consume, consume,
                        "case={case_index} step={step_index}"
                    );
                }
            }
        }
        (specs.len(), cases.len())
    }

    #[test]
    fn external_replay_isolates_clocks_and_checks_mouse_context_and_injection() {
        let document = json!({
            "config":{"version":17,"bindings":[
                {"id":1,"held":[{"kind":"key","value":32}],"activation":{"kind":"press","value":{"kind":"key","value":75}},
                 "keyboard":{"repeat":true,"retained_prefix":{"key":{"kind":"key","value":32},"quiet_ms":50,"fast_ms":50},"matching":{"kind":"captured","priority":0},"restore_on_tap":true,"tap_timeout_ms":1500},
                 "message":"回放协议测试","enabled":false,"action":{"kind":"notification"}},
                {"id":2,"held":[{"kind":"x1"}],"activation":{"kind":"wheel_up"},"wheel":"packet",
                 "pointer_scope":{"allow":["reader"]},"message":"指针回放协议测试","enabled":false,"action":{"kind":"notification"}}
            ]},
            "cases":[
                {"reset":true,"process":"editor","steps":[
                    {"key":32,"down":true,"time_ms":100,"triggered":[],"consume":true},
                    {"key":32,"down":false,"time_ms":20000,"injected":true,"triggered":[],"consume":false},
                    {"key":75,"down":true,"time_ms":150,"triggered":[1],"consume":true},
                    {"event":{"kind":"foreground","process":"other","role":"application"},"triggered":[],"consume":false},
                    {"key":75,"down":false,"time_ms":160,"triggered":[],"consume":true},
                    {"key":32,"down":false,"time_ms":170,"triggered":[],"consume":true}
                ]},
                {"reset":true,"process":"editor","steps":[
                    {"key":32,"down":true,"time_ms":100,"triggered":[],"consume":true},
                    {"key":75,"down":true,"time_ms":149,"triggered":[],"consume":false,"replay":[{"key":32,"down":true}]},
                    {"key":32,"down":false,"time_ms":200,"triggered":[],"consume":false},
                    {"key":75,"down":false,"time_ms":201,"triggered":[],"consume":false}
                ]},
                {"reset":true,"process":"editor","pointer_process":"reader","pointer_role":"application","steps":[
                    {"event":{"kind":"down","button":{"kind":"x1"}},"time_ms":0,"triggered":[],"consume":true},
                    {"event":{"kind":"wheel","axis":"vertical","delta":1},"time_ms":1,"triggered":[2],"consume":true},
                    {"event":{"kind":"pointer","process":null,"role":"unknown"},"triggered":[],"consume":false},
                    {"event":{"kind":"wheel","axis":"vertical","delta":1},"time_ms":2,"triggered":[],"consume":false},
                    {"event":{"kind":"up","button":{"kind":"x1"}},"time_ms":3,"triggered":[],"consume":true}
                ]},
                {"reset":true,"process":"editor","pointer_process":"reader","pointer_role":"application","steps":[
                    {"event":{"kind":"down","button":{"kind":"x1"}},"triggered":[],"consume":true},
                    {"event":{"kind":"pointer","process":null,"role":"unknown"},"triggered":[],"consume":false,
                     "replay":[{"kind":"down","button":{"kind":"x1"}}]},
                    {"event":{"kind":"up","button":{"kind":"x1"}},"triggered":[],"consume":false}
                ]},
                {"reset":true,"process":"editor","steps":[
                    {"key":32,"down":true,"time_ms":100,"triggered":[],"consume":true},
                    {"event":{"kind":"advance"},"time_ms":1601,"triggered":[],"consume":false},
                    {"key":32,"down":false,"triggered":[],"consume":true},
                    {"key":32,"down":true,"time_ms":1700,"triggered":[],"consume":true},
                    {"event":{"kind":"cancel"},"triggered":[],"consume":false},
                    {"key":32,"down":false,"time_ms":1701,"triggered":[],"consume":true}
                ]}
            ]
        });
        assert_eq!(replay_document(&document), (2, 5));
        assert!(serde_json::from_value::<ReplayEvent>(
            json!({"kind":"wheel","axis":"diagonal","delta":1})
        )
        .is_err());
        assert!(
            serde_json::from_value::<ReplayEvent>(json!({"kind":"cancel","extra":true})).is_err()
        );
    }

    #[test]
    #[ignore = "读取显式指定的本机回放样本，仅执行纯核心，不安装 Hook 或运行动作"]
    fn captured_matching_replays_external_source_witnesses() {
        let path = std::env::var_os("OMNITRIGGER_CAPTURE_REPLAY").expect("需要指定回放样本");
        let file = std::fs::File::open(path).unwrap();
        assert!(file.metadata().unwrap().len() <= 32 * 1024 * 1024);
        let document: serde_json::Value = serde_json::from_reader(file).unwrap();
        let (rules, cases) = replay_document(&document);
        println!("捕获集合回放通过：{rules} 规则、{cases} 组序列；没有安装 Hook 或执行动作。");
    }

    #[test]
    fn process_scope_round_trip_keeps_user_names_and_rejects_legacy_schema() {
        let mut bindings = vec![defaults()[0].clone()];
        let mut scoped = bindings[0].clone();
        scoped.spec.id = 88;
        scoped.spec.process_scope = ProcessScope {
            identity: Default::default(),
            allow: vec!["Code.EXE".into(), "cursor".into()],
            deny: vec!["cursor.exe".into()],
        };
        bindings.push(scoped);
        let text = encode(&bindings);
        assert_eq!(decode(&text).unwrap(), bindings);
        let mut document: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(document["bindings"][0].get("process_scope").is_none());
        document["version"] = 9.into();
        assert!(decode(&document.to_string())
            .unwrap_err()
            .contains("版本 10"));
        document["version"] = 10.into();
        document["bindings"][1]["process_scope"]["allow"] = json!(["Code", "code.exe"]);
        assert!(decode(&document.to_string())
            .unwrap_err()
            .contains("重复名称"));
    }

    #[test]
    fn process_scope_upgrade_backs_up_exact_v9_content_before_saving() {
        let scratch = Scratch::new();
        let destination = scratch.config();
        let mut document: serde_json::Value = serde_json::from_str(&encode(&defaults())).unwrap();
        document["version"] = 9.into();
        let original = format!(" \r\n{document}\r\n");
        std::fs::write(&destination, &original).unwrap();
        let mut bindings = defaults();
        bindings[0].spec.process_scope = ProcessScope {
            identity: Default::default(),
            allow: vec![],
            deny: vec!["game.exe".into()],
        };
        save_to(&destination, &bindings).unwrap();
        assert_eq!(
            std::fs::read_to_string(scratch.0.join("bindings.json.v9.bak")).unwrap(),
            original
        );
        assert_eq!(
            decode(&std::fs::read_to_string(&destination).unwrap()).unwrap(),
            bindings
        );
        let saved = std::fs::read(&destination).unwrap();
        bindings[0].spec.process_scope.allow = vec!["bad/path".into()];
        assert!(save_to(&destination, &bindings).is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), saved);
    }

    #[test]
    fn browser_actions_upgrade_v8_with_exact_backup_and_reject_mislabeled_schema() {
        let scratch = Scratch::new();
        let destination = scratch.config();
        let mut previous: serde_json::Value = serde_json::from_str(&encode(&defaults())).unwrap();
        previous["version"] = 8.into();
        let original = format!(" \r\n{previous}\r\n");
        std::fs::write(&destination, &original).unwrap();
        let mut bindings = defaults();
        bindings[0].action = ActionSpec::Browser(crate::browser_actions::BrowserAction {
            command: crate::browser_actions::BrowserCommand::Call {
                method: "bookmarks.search".into(),
                args: vec![json!({"query":"中文书签"})],
            },
            profile: Some("work-browser".into()),
        });
        save_to(&destination, &bindings).unwrap();
        assert_eq!(
            std::fs::read_to_string(scratch.0.join("bindings.json.v8.bak")).unwrap(),
            original
        );
        assert_eq!(
            decode(&std::fs::read_to_string(&destination).unwrap()).unwrap(),
            bindings
        );
        let mut old: serde_json::Value = serde_json::from_str(&encode(&bindings)).unwrap();
        old["version"] = 8.into();
        assert_eq!(
            decode(&old.to_string()).unwrap_err(),
            "浏览器动作需要版本 9 配置。"
        );
    }

    #[test]
    fn package_calls_preserve_uninstalled_identity_and_input_and_require_v8() {
        let mut expected = defaults();
        expected[0].action = ActionSpec::Invoke(omnitrigger_actions::ActionCall {
            package_id: "missing.package".into(),
            action_id: "run".into(),
            action_version: 12,
            input: json!({"program":"C:\\Program Files\\工具.exe", "args":["有 空格", "${literal}"], "nested":{"enabled":true}}),
        });
        let encoded = encode(&expected);
        assert_eq!(decode(&encoded).unwrap(), expected);
        let mut legacy: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        legacy["version"] = 7.into();
        assert!(decode(&legacy.to_string()).unwrap_err().contains("版本 8"));
        let scratch = Scratch::new();
        let destination = scratch.config();
        let mut old: serde_json::Value = serde_json::from_str(&encode(&defaults())).unwrap();
        old["version"] = 7.into();
        let original = format!(" \r\n{old}\r\n");
        std::fs::write(&destination, &original).unwrap();
        save_to(&destination, &expected).unwrap();
        assert_eq!(
            std::fs::read_to_string(scratch.0.join("bindings.json.v7.bak")).unwrap(),
            original
        );
        assert_eq!(
            decode(&std::fs::read_to_string(&destination).unwrap()).unwrap(),
            expected
        );
        expected[0].action = ActionSpec::Invoke(omnitrigger_actions::ActionCall {
            package_id: "missing.package".into(),
            action_id: "run".into(),
            action_version: 12,
            input: json!([]),
        });
        assert!(validate(&expected).is_err());
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
                ActionSpec::SendShortcut(ShortcutAction::new(vec![output]))
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
    fn current_round_trip_preserves_all_action_variants_and_unavailable_plugin_parameters() {
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
            ActionSpec::SendShortcut(ShortcutAction::new(vec![8]))
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
                assert!(actual[0].spec.conditions.is_empty());
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
        assert!(decode(&format!(
            r#"{{"version":{},"bindings":[]}}"#,
            CONFIG_VERSION + 1
        ))
        .is_err());
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
    fn save_backs_up_legacy_bytes_once_then_saves_current_version() {
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

    fn drag_binding(button: Button, min_distance_px: u32, max_start_ms: u32) -> Binding {
        Binding {
            spec: TriggerSpec {
                pointer_scope: Default::default(),
                wheel: Default::default(),
                keyboard: Default::default(),
                process_scope: Default::default(),
                conditions: Vec::new(),
                id: 6,
                held: vec![],
                activation: Activation::Drag {
                    button,
                    min_distance_px,
                    max_start_ms,
                },
            },
            message: "快速拖拽已触发".into(),
            enabled: true,
            action: ActionSpec::Notification,
        }
    }

    #[test]
    fn drag_dto_round_trip_keeps_current_version_parameters_and_independent_action() {
        let drag = drag_binding(Button::Mouse(MouseButton::Right), 18, 180);
        let mut bindings = defaults();
        bindings.push(drag.clone());
        validate(&bindings).unwrap();
        assert_eq!(decode(&encode(&bindings)).unwrap(), bindings);
        let serialized = serde_json::to_value(BindingDto::from(&drag)).unwrap();
        assert_eq!(
            serialized["activation"],
            json!({
                "kind": "drag",
                "value": {
                    "button": {"kind": "right"},
                    "min_distance_px": 18,
                    "max_start_ms": 180
                }
            })
        );
        assert_eq!(serialized["action"], json!({"kind": "notification"}));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&encode(&bindings)).unwrap()["version"],
            CONFIG_VERSION
        );
        assert_eq!(bindings.last(), Some(&drag));
    }

    #[test]
    fn drag_parameters_and_hold_timing_conflicts_are_reported_without_rewriting() {
        for (button, distance, window, title) in [
            (Button::Key(0x41), 18, 180, "拖拽目标必须是鼠标按钮"),
            (Button::Mouse(MouseButton::Right), 3, 180, "拖拽距离无效"),
            (Button::Mouse(MouseButton::Right), 501, 180, "拖拽距离无效"),
            (Button::Mouse(MouseButton::Right), 18, 19, "快拖时窗无效"),
            (Button::Mouse(MouseButton::Right), 18, 2001, "快拖时窗无效"),
        ] {
            let drag = drag_binding(button, distance, window);
            let original = drag.clone();
            assert!(decode(&encode(std::slice::from_ref(&drag))).is_err());
            assert_eq!(check_candidate(&[], &drag.spec).unwrap_err().title, title);
            assert_eq!(drag, original);
        }
        let drag = drag_binding(Button::Mouse(MouseButton::Right), 18, 300);
        let conflict = check_candidate(&defaults(), &drag.spec).unwrap_err();
        assert_eq!(conflict.title, "长按与快拖时序冲突");
        assert_eq!(conflict.other, Some(5));
        assert_eq!(
            drag.spec.activation,
            Activation::Drag {
                button: Button::Mouse(MouseButton::Right),
                min_distance_px: 18,
                max_start_ms: 300,
            }
        );
        let left = drag_binding(Button::Mouse(MouseButton::Left), 18, 180);
        assert!(check_candidate(&defaults(), &left.spec).is_ok());
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
        assert_eq!(duplicate.title, "与已有触发范围冲突");
        let prefix = TriggerSpec {
            pointer_scope: Default::default(),
            wheel: Default::default(),
            keyboard: Default::default(),
            process_scope: Default::default(),
            conditions: Vec::new(),
            id: 0,
            held: vec![Button::Key(0xa0)],
            activation: Activation::Press(Button::Key(0x14)),
        };
        // Shift+CapsLock 与 CapsLock+W 的 held 可区分，即使已有项禁用也不误报。
        check_candidate(&values, &prefix).unwrap();
        // 裸 CapsLock 会抢先作为动作末键，仍与已有 CapsLock 前缀冲突。
        let prefix = TriggerSpec {
            held: vec![],
            ..prefix
        };
        assert_eq!(
            check_candidate(&values, &prefix).unwrap_err().other,
            Some(1)
        );
        let bare_left = TriggerSpec {
            pointer_scope: Default::default(),
            wheel: Default::default(),
            keyboard: Default::default(),
            process_scope: Default::default(),
            conditions: Vec::new(),
            id: 0,
            held: vec![],
            activation: Activation::Press(Button::Mouse(MouseButton::Left)),
        };
        assert_eq!(
            check_candidate(&values, &bare_left).unwrap_err().other,
            None
        );
        let fresh = TriggerSpec {
            pointer_scope: Default::default(),
            wheel: Default::default(),
            keyboard: Default::default(),
            process_scope: Default::default(),
            conditions: Vec::new(),
            id: 0,
            held: vec![Button::Key(0x14)],
            activation: Activation::Press(Button::Key(0x45)),
        };
        assert!(check_candidate(&values, &fresh).is_ok());
        assert_eq!(values.len(), 5);
    }

    fn version_five(bindings: &[Binding]) -> String {
        let mut document: serde_json::Value = serde_json::from_str(&encode(bindings)).unwrap();
        document["version"] = 5.into();
        document.to_string()
    }

    fn regional_binding(
        id: u64,
        region: PointerRegion,
        direction: WheelDirection,
        target: AdjustmentTarget,
        delta_percent: i16,
    ) -> Binding {
        Binding {
            spec: TriggerSpec {
                pointer_scope: Default::default(),
                wheel: Default::default(),
                keyboard: Default::default(),
                process_scope: Default::default(),
                id,
                held: vec![],
                conditions: vec![TriggerCondition::PointerRegion(region)],
                activation: Activation::Wheel(direction),
            },
            message: format!("区域动作 {id}"),
            enabled: true,
            action: ActionSpec::AdjustSystem(SystemAdjustmentAction {
                target,
                delta_percent,
            }),
        }
    }

    #[test]
    fn version_five_preserves_every_existing_action_and_identity_with_no_position_condition() {
        let mut plugin = PluginActionRef::raishot_capture();
        plugin.plugin_id = "missing-plugin".into();
        plugin
            .parameters
            .insert("原参数".into(), json!({"nested":[42,"中文"]}));
        let actions = vec![
            ActionSpec::Notification,
            ActionSpec::ShowPanel,
            ActionSpec::ShowWheel(WheelActionRef {
                wheel_id: "missing-wheel".into(),
            }),
            ActionSpec::Command(CommandAction {
                program: "工具.exe".into(),
                args: vec!["中文 空格 & 符号".into()],
                working_directory: Some("C:\\旧目录".into()),
            }),
            ActionSpec::SendShortcut(ShortcutAction::parse("RightCtrl+Backspace").unwrap()),
            ActionSpec::Window(WindowAction {
                operation: crate::window_action::WindowOperation::Minimize,
            }),
            ActionSpec::External(ExternalAction {
                provider: "旧依赖".into(),
                command: CommandAction {
                    program: std::env::current_exe()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    args: vec!["旧动作参数".into()],
                    working_directory: None,
                },
                disabled_reason: Some("原动作不可用".into()),
            }),
            ActionSpec::Plugin(plugin),
        ];
        let expected: Vec<_> = actions
            .into_iter()
            .enumerate()
            .map(|(index, action)| Binding {
                spec: TriggerSpec {
                    pointer_scope: Default::default(),
                    wheel: Default::default(),
                    keyboard: Default::default(),
                    process_scope: Default::default(),
                    id: 100 + index as u64,
                    held: vec![Button::Key(0x14)],
                    conditions: vec![],
                    activation: Activation::Press(Button::Key(0x70 + index as u16)),
                },
                message: format!("旧动作 {index}"),
                enabled: index % 2 == 0,
                action,
            })
            .collect();
        let original = version_five(&expected);
        assert!(!original.contains("conditions"));
        let actual = decode(&original).unwrap();
        assert_eq!(actual, expected);
        assert!(actual
            .iter()
            .all(|binding| binding.spec.conditions.is_empty()));
        assert_eq!(decode(&encode(&actual)).unwrap(), expected);
    }

    #[test]
    fn version_six_preserves_regions_rectangle_and_signed_system_adjustments_on_round_trip() {
        let expected = vec![
            regional_binding(
                10,
                PointerRegion::ScreenEdge {
                    edge: ScreenEdge::Top,
                    thickness_px: 8,
                },
                WheelDirection::Up,
                AdjustmentTarget::Volume,
                5,
            ),
            regional_binding(
                11,
                PointerRegion::ScreenEdge {
                    edge: ScreenEdge::Bottom,
                    thickness_px: 12,
                },
                WheelDirection::Down,
                AdjustmentTarget::Volume,
                -7,
            ),
            regional_binding(
                12,
                PointerRegion::ScreenCorner {
                    corner: ScreenCorner::BottomRight,
                    size_px: 40,
                },
                WheelDirection::Left,
                AdjustmentTarget::Brightness,
                10,
            ),
            regional_binding(
                13,
                PointerRegion::Rectangle(ScreenRect {
                    left: -512,
                    top: -180,
                    right: -1,
                    bottom: 400,
                }),
                WheelDirection::Right,
                AdjustmentTarget::Brightness,
                -100,
            ),
        ];
        let text = encode(&expected);
        assert_eq!(decode(&text).unwrap(), expected);
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["version"], CONFIG_VERSION);
        assert_eq!(
            value["bindings"][0]["conditions"],
            json!([{"kind":"pointer_region","region":{"kind":"screen_edge","edge":"top","thickness_px":8}}])
        );
        assert_eq!(
            value["bindings"][2]["conditions"][0]["region"],
            json!({"kind":"screen_corner","corner":"bottom_right","size_px":40})
        );
        assert_eq!(
            value["bindings"][3]["conditions"][0]["region"],
            json!({"kind":"rectangle","left":-512,"top":-180,"right":-1,"bottom":400})
        );
        assert_eq!(
            value["bindings"][1]["action"],
            json!({"kind":"adjust_system","target":"volume","delta_percent":-7})
        );
        assert_eq!(
            value["bindings"][3]["action"],
            json!({"kind":"adjust_system","target":"brightness","delta_percent":-100})
        );
    }

    #[test]
    fn screen_edge_segments_round_trip_and_require_version_eight() {
        let scratch = Scratch::new();
        let destination = scratch.config();
        let expected: Vec<_> = [
            (ScreenEdge::Top, WheelDirection::Up),
            (ScreenEdge::Right, WheelDirection::Down),
            (ScreenEdge::Bottom, WheelDirection::Left),
            (ScreenEdge::Left, WheelDirection::Right),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (edge, direction))| {
            regional_binding(
                80 + index as u64,
                PointerRegion::ScreenEdgeSegment {
                    edge,
                    thickness_px: 8,
                    start_inset_px: 8,
                    end_inset_px: 13,
                },
                direction,
                AdjustmentTarget::Brightness,
                -5,
            )
        })
        .collect();
        let encoded = encode(&expected);
        assert_eq!(decode(&encoded).unwrap(), expected);
        let document: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(document["version"], CONFIG_VERSION);
        assert_eq!(
            document["bindings"][3]["conditions"][0]["region"],
            json!({
                "kind":"screen_edge_segment", "edge":"left", "thickness_px":8,
                "start_inset_px":8, "end_inset_px":13
            })
        );
        for version in 6..=7 {
            let mut old = document.clone();
            old["version"] = version.into();
            let original = old.to_string();
            assert_eq!(decode(&original).unwrap_err(), "边缘区段需要版本 8 配置。");
            std::fs::write(&destination, &original).unwrap();
            assert!(save_to(&destination, &expected).is_err());
            assert_eq!(std::fs::read_to_string(&destination).unwrap(), original);
        }
    }

    #[test]
    fn trimmed_sides_and_top_can_share_both_wheel_directions_and_preserve_existing_bindings() {
        let mut expected = defaults();
        for (edge, target) in [
            (ScreenEdge::Left, AdjustmentTarget::Brightness),
            (ScreenEdge::Right, AdjustmentTarget::Brightness),
            (ScreenEdge::Top, AdjustmentTarget::Volume),
        ] {
            let region = if edge == ScreenEdge::Top {
                PointerRegion::ScreenEdge {
                    edge,
                    thickness_px: 8,
                }
            } else {
                PointerRegion::ScreenEdgeSegment {
                    edge,
                    thickness_px: 8,
                    start_inset_px: 8,
                    end_inset_px: 0,
                }
            };
            for (direction, delta_percent) in [(WheelDirection::Up, 5), (WheelDirection::Down, -5)]
            {
                let binding = regional_binding(
                    90 + expected.len() as u64,
                    region,
                    direction,
                    target,
                    delta_percent,
                );
                check_candidate(&expected, &binding.spec).unwrap();
                expected.push(binding);
            }
        }
        validate(&expected).unwrap();
        assert_eq!(decode(&encode(&expected)).unwrap(), expected);
        assert_eq!(&expected[..defaults().len()], defaults());
    }

    #[test]
    fn opposite_edges_can_share_a_wheel_direction_but_overlapping_edge_and_corner_cannot() {
        let left = regional_binding(
            30,
            PointerRegion::ScreenEdge {
                edge: ScreenEdge::Left,
                thickness_px: 8,
            },
            WheelDirection::Up,
            AdjustmentTarget::Volume,
            5,
        );
        let right = regional_binding(
            31,
            PointerRegion::ScreenEdge {
                edge: ScreenEdge::Right,
                thickness_px: 8,
            },
            WheelDirection::Up,
            AdjustmentTarget::Brightness,
            5,
        );
        let distinct = vec![left.clone(), right.clone()];
        validate(&distinct).unwrap();
        check_candidate(std::slice::from_ref(&left), &right.spec).unwrap();
        assert_eq!(decode(&encode(&distinct)).unwrap(), distinct);

        let corner = regional_binding(
            32,
            PointerRegion::ScreenCorner {
                corner: ScreenCorner::TopLeft,
                size_px: 32,
            },
            WheelDirection::Up,
            AdjustmentTarget::Brightness,
            -5,
        );
        assert!(decode(&encode(&[left.clone(), corner.clone()])).is_err());
        let conflict = check_candidate(std::slice::from_ref(&left), &corner.spec).unwrap_err();
        assert_eq!(conflict.other, Some(left.spec.id));
        assert_eq!(conflict.title, "与已有触发范围冲突");

        let mut unconditional = right;
        unconditional.spec.conditions.clear();
        assert!(check_candidate(&[left], &unconditional.spec).is_err());
    }

    #[test]
    fn invalid_adjustment_and_region_candidates_never_replace_a_valid_original() {
        let scratch = Scratch::new();
        let destination = scratch.config();
        let valid = regional_binding(
            40,
            PointerRegion::ScreenEdge {
                edge: ScreenEdge::Top,
                thickness_px: 8,
            },
            WheelDirection::Up,
            AdjustmentTarget::Volume,
            5,
        );
        let original = encode(std::slice::from_ref(&valid));
        std::fs::write(&destination, &original).unwrap();
        let mut invalid_candidates = Vec::new();
        for delta_percent in [0, -101, 101, i16::MIN, i16::MAX] {
            let mut candidate = valid.clone();
            candidate.action = ActionSpec::AdjustSystem(SystemAdjustmentAction {
                target: AdjustmentTarget::Volume,
                delta_percent,
            });
            invalid_candidates.push(candidate);
        }
        for region in [
            PointerRegion::ScreenEdge {
                edge: ScreenEdge::Top,
                thickness_px: 0,
            },
            PointerRegion::ScreenEdge {
                edge: ScreenEdge::Right,
                thickness_px: 257,
            },
            PointerRegion::ScreenCorner {
                corner: ScreenCorner::TopLeft,
                size_px: 0,
            },
            PointerRegion::ScreenCorner {
                corner: ScreenCorner::BottomRight,
                size_px: 1025,
            },
            PointerRegion::Rectangle(ScreenRect {
                left: 20,
                top: 0,
                right: 20,
                bottom: 100,
            }),
            PointerRegion::Rectangle(ScreenRect {
                left: 0,
                top: 20,
                right: 100,
                bottom: 19,
            }),
        ] {
            let mut candidate = valid.clone();
            candidate.spec.conditions = vec![TriggerCondition::PointerRegion(region)];
            invalid_candidates.push(candidate);
        }
        for candidate in invalid_candidates {
            assert!(decode(&encode(std::slice::from_ref(&candidate))).is_err());
            assert!(save_to(&destination, &[candidate]).is_err());
            assert_eq!(std::fs::read_to_string(&destination).unwrap(), original);
        }
        assert_eq!(std::fs::read_dir(&scratch.0).unwrap().count(), 1);
    }

    #[test]
    fn unknown_condition_fields_and_legacy_new_fields_are_rejected_without_rewriting_source() {
        let scratch = Scratch::new();
        let destination = scratch.config();
        let valid = regional_binding(
            50,
            PointerRegion::ScreenEdge {
                edge: ScreenEdge::Top,
                thickness_px: 8,
            },
            WheelDirection::Up,
            AdjustmentTarget::Volume,
            5,
        );
        let base: serde_json::Value =
            serde_json::from_str(&encode(std::slice::from_ref(&valid))).unwrap();
        let mut invalid_documents = Vec::new();
        let mut unknown_condition = base.clone();
        unknown_condition["bindings"][0]["conditions"][0]["source"] = "physical".into();
        invalid_documents.push(unknown_condition);
        let mut unknown_region = base.clone();
        unknown_region["bindings"][0]["conditions"][0]["region"]["exclude_connected"] =
            false.into();
        invalid_documents.push(unknown_region);
        let mut unknown_kind = base.clone();
        unknown_kind["bindings"][0]["conditions"][0]["kind"] = "future-condition".into();
        invalid_documents.push(unknown_kind);
        let mut unknown_action = base.clone();
        unknown_action["bindings"][0]["action"]["absolute_percent"] = 50.into();
        invalid_documents.push(unknown_action);
        let mut old_with_condition = base.clone();
        old_with_condition["version"] = 5.into();
        old_with_condition["bindings"][0]["action"] = json!({"kind":"notification"});
        invalid_documents.push(old_with_condition);
        let mut old_with_adjustment = base;
        old_with_adjustment["version"] = 5.into();
        old_with_adjustment["bindings"][0]
            .as_object_mut()
            .unwrap()
            .remove("conditions");
        invalid_documents.push(old_with_adjustment);
        for document in invalid_documents {
            let original = document.to_string();
            assert!(decode(&original).is_err());
            std::fs::write(&destination, &original).unwrap();
            assert!(save_to(&destination, std::slice::from_ref(&valid)).is_err());
            assert_eq!(std::fs::read_to_string(&destination).unwrap(), original);
        }
        assert_eq!(std::fs::read_dir(&scratch.0).unwrap().count(), 1);
    }

    #[test]
    fn saving_version_five_makes_one_exact_byte_backup_and_upgrades_to_six() {
        let scratch = Scratch::new();
        let destination = scratch.config();
        let backup = scratch.0.join("bindings.json.v5.bak");
        let original = format!(" \r\n{}\r\n ", version_five(&defaults()));
        std::fs::write(&destination, original.as_bytes()).unwrap();
        let mut bindings = decode(&original).unwrap();
        bindings.push(regional_binding(
            70,
            PointerRegion::ScreenEdge {
                edge: ScreenEdge::Top,
                thickness_px: 8,
            },
            WheelDirection::Up,
            AdjustmentTarget::Volume,
            5,
        ));
        save_to(&destination, &bindings).unwrap();
        assert_eq!(std::fs::read(&backup).unwrap(), original.as_bytes());
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&destination).unwrap()).unwrap();
        assert_eq!(saved["version"], CONFIG_VERSION);
        assert_eq!(decode(&saved.to_string()).unwrap(), bindings);
        bindings.last_mut().unwrap().message = "修改后的边缘音量".into();
        save_to(&destination, &bindings).unwrap();
        assert_eq!(std::fs::read(&backup).unwrap(), original.as_bytes());
        assert_eq!(
            decode(&std::fs::read_to_string(&destination).unwrap()).unwrap(),
            bindings
        );
        assert_eq!(std::fs::read_dir(&scratch.0).unwrap().count(), 2);
    }

    fn local_binding() -> Binding {
        let mut binding = defaults().remove(0);
        binding.message = "本地动作 中文与模板".into();
        binding.enabled = false;
        binding.action = ActionSpec::Local(LocalAction {
            version: 1,
            steps: vec![
                LocalStep::SetVariable {
                    name: "prefix".into(),
                    value: "中文 $$".into(),
                },
                LocalStep::ReadClipboard {
                    variable: "clip".into(),
                },
                LocalStep::InputText {
                    text: "${prefix}\r\n${clip}".into(),
                },
                LocalStep::SetClipboard {
                    text: "$${literal} ${clip}".into(),
                },
                LocalStep::SendShortcut {
                    keys: vec![0xa2, 0x41],
                },
                LocalStep::KeyTap { key: 0x12 },
                LocalStep::KeySequence {
                    modifiers: vec![0xa2],
                    keys: vec![0x4b, 0x5a],
                },
                LocalStep::Delay { milliseconds: 125 },
                LocalStep::Open {
                    target: "https://example.com".into(),
                },
            ],
        });
        binding
    }

    #[test]
    fn local_actions_round_trip_in_v7_and_older_versions_are_preserved_on_rejection() {
        let scratch = Scratch::new();
        let destination = scratch.config();
        let expected = vec![local_binding()];
        save_to(&destination, &expected).unwrap();
        let saved = std::fs::read_to_string(&destination).unwrap();
        assert_eq!(decode(&saved).unwrap(), expected);
        let current: serde_json::Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(current["version"], CONFIG_VERSION);
        assert_eq!(current["bindings"][0]["action"]["kind"], "local");
        assert_eq!(current["bindings"][0]["action"]["version"], 1);
        for version in 1..=6 {
            let mut old = current.clone();
            old["version"] = version.into();
            let original = old.to_string();
            let error = decode(&original).unwrap_err();
            if version >= 5 {
                assert!(error.contains("版本 7"), "错误的拒绝原因：{error}");
            }
            std::fs::write(&destination, original.as_bytes()).unwrap();
            assert!(save_to(&destination, &expected).is_err());
            assert_eq!(std::fs::read(&destination).unwrap(), original.as_bytes());
        }
        assert_eq!(std::fs::read_dir(&scratch.0).unwrap().count(), 1);
    }

    #[test]
    fn adding_local_action_to_v6_keeps_exact_legacy_backup_and_saves_v7() {
        let scratch = Scratch::new();
        let destination = scratch.config();
        let backup = scratch.0.join("bindings.json.v6.bak");
        let mut legacy: serde_json::Value = serde_json::from_str(&encode(&defaults())).unwrap();
        legacy["version"] = 6.into();
        let original = format!(" \r\n{}\r\n ", legacy);
        std::fs::write(&destination, original.as_bytes()).unwrap();
        let mut expected = decode(&original).unwrap();
        expected[0] = local_binding();
        save_to(&destination, &expected).unwrap();
        assert_eq!(std::fs::read(&backup).unwrap(), original.as_bytes());
        let saved = std::fs::read_to_string(&destination).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&saved).unwrap()["version"],
            CONFIG_VERSION
        );
        assert_eq!(decode(&saved).unwrap(), expected);
        expected[0].message = "再次修改本地动作".into();
        save_to(&destination, &expected).unwrap();
        assert_eq!(std::fs::read(&backup).unwrap(), original.as_bytes());
        assert_eq!(
            decode(&std::fs::read_to_string(&destination).unwrap()).unwrap(),
            expected
        );
        assert_eq!(std::fs::read_dir(&scratch.0).unwrap().count(), 2);
    }
}
