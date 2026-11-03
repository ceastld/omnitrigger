//! 注册限制、冲突检查与结构化错误。

use std::collections::BTreeSet;
use std::fmt;

use crate::{
    Activation, Button, ProcessScopeError, RegionError, TriggerCondition, TriggerSpec, regions,
};

/// Hook 匹配仍受固定上限约束；完整检查发生在注册时。
pub const MAX_BINDINGS: usize = 512;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistrationError {
    InvalidDedicatedPrefix {
        id: u64,
        button: Button,
    },
    InvalidKeyboardRepeat {
        id: u64,
    },
    InvalidTapRestore {
        id: u64,
    },
    InvalidTapProcessScope {
        id: u64,
        reason: ProcessScopeError,
    },
    ConflictingPrefixPolicy {
        first: u64,
        second: u64,
        button: Button,
    },
    InvalidProcessScope {
        id: u64,
        reason: ProcessScopeError,
    },
    TooManyBindings,
    TooManyConditions {
        id: u64,
    },
    InvalidPointerRegion {
        id: u64,
        reason: RegionError,
    },
    TooManyHeld {
        id: u64,
    },
    InvalidButton {
        id: u64,
        button: Button,
    },
    InvalidHoldDuration {
        id: u64,
        duration_ms: u32,
    },
    InvalidDragButton {
        id: u64,
        button: Button,
    },
    InvalidDragDistance {
        id: u64,
        min_distance_px: u32,
    },
    InvalidDragWindow {
        id: u64,
        max_start_ms: u32,
    },
    TimingConflict {
        hold_binding: u64,
        drag_binding: u64,
        hold_duration_ms: u32,
        drag_max_start_ms: u32,
    },
    DuplicateId {
        id: u64,
    },
    DuplicateHeld {
        id: u64,
    },
    TerminalInHeld {
        id: u64,
    },
    DuplicateTrigger {
        first: u64,
        second: u64,
    },
    TerminalIsPrefix {
        terminal_binding: u64,
        prefix_binding: u64,
    },
}

impl fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidDedicatedPrefix { .. } => {
                "专用前缀必须是当前组合中按住的明确键盘键，不能使用鼠标键或泛型修饰键。"
            }
            Self::InvalidKeyboardRepeat { .. } => {
                "键盘重复仅支持键盘按下触发，不支持鼠标、长按、拖拽或滚轮。"
            }
            Self::InvalidTapRestore { .. } => {
                "单按恢复需要专用前缀；超时仅在启用恢复时可设置，应为 1 至 60000 毫秒。"
            }
            Self::InvalidTapProcessScope { reason, .. } => {
                return write!(f, "单按恢复的应用范围无效：{reason}");
            }
            Self::ConflictingPrefixPolicy { .. } => {
                "重叠应用和位置范围内，同一前缀的专用层、单按恢复与超时策略必须一致。"
            }
            Self::InvalidProcessScope { reason, .. } => return write!(f, "进程范围无效：{reason}"),
            Self::TooManyBindings => {
                return write!(
                    f,
                    "最多可添加 {MAX_BINDINGS} 个触发器，请删除不再使用的触发器后重试。"
                );
            }
            Self::TooManyConditions { .. } => "每个触发器目前最多配置一个鼠标区域条件。",
            Self::InvalidPointerRegion { reason, .. } => {
                return write!(f, "鼠标区域条件无效：{reason}");
            }
            Self::TooManyHeld { .. } => "每个组合最多包含 8 个前缀按键，请减少同时按住的按键。",
            Self::InvalidButton { .. } => "组合包含不支持的按键，请重新录制。",
            Self::InvalidHoldDuration { .. } => "长按时间应为 50 至 10000 毫秒。",
            Self::InvalidDragButton { .. } => "快速拖拽的目标必须是鼠标按键。",
            Self::InvalidDragDistance { .. } => "快速拖拽距离应为 4 至 500 像素。",
            Self::InvalidDragWindow { .. } => "快速拖拽起始窗口应为 20 至 2000 毫秒。",
            Self::TimingConflict { .. } => "同组合的快速拖拽窗口必须严格早于长按阈值。",
            Self::DuplicateId { .. } => "触发器编号重复，请重新添加该触发器。",
            Self::DuplicateHeld { .. } => "组合中的前缀按键重复，请重新录制。",
            Self::TerminalInHeld { .. } => "目标键不能同时作为当前组合的前缀，请调整组合后重试。",
            Self::DuplicateTrigger { .. } => "已存在相同组合，请编辑已有触发器。",
            Self::TerminalIsPrefix { .. } => {
                "目标键已作为另一组合的前缀，请调整组合或删除冲突触发器。"
            }
        })
    }
}
impl std::error::Error for RegistrationError {}

pub(super) fn validate(
    mut bindings: Vec<TriggerSpec>,
) -> Result<Vec<TriggerSpec>, RegistrationError> {
    if bindings.len() > MAX_BINDINGS {
        return Err(RegistrationError::TooManyBindings);
    }
    let mut ids = BTreeSet::new();
    for binding in &mut bindings {
        if let Some(button) = binding.keyboard.dedicated_prefix {
            if !matches!(button, Button::Key(key) if (1..=254).contains(&key) && !matches!(key, 0x10..=0x12))
                || !binding.held.contains(&button)
            {
                return Err(RegistrationError::InvalidDedicatedPrefix {
                    id: binding.id,
                    button,
                });
            }
        }
        if binding.keyboard.repeat
            && !matches!(binding.activation, Activation::Press(Button::Key(_)))
        {
            return Err(RegistrationError::InvalidKeyboardRepeat { id: binding.id });
        }
        if ((binding.keyboard.restore_on_tap || binding.keyboard.tap_timeout_ms.is_some())
            && binding.keyboard.dedicated_prefix.is_none())
            || (binding.keyboard.tap_timeout_ms.is_some() && !binding.keyboard.restore_on_tap)
            || binding
                .keyboard
                .tap_timeout_ms
                .is_some_and(|ms| !(1..=60_000).contains(&ms))
        {
            return Err(RegistrationError::InvalidTapRestore { id: binding.id });
        }
        if !binding.keyboard.restore_on_tap && !binding.keyboard.tap_scope.is_global() {
            return Err(RegistrationError::InvalidTapRestore { id: binding.id });
        }
        binding.keyboard.tap_scope.normalize().map_err(|reason| {
            RegistrationError::InvalidTapProcessScope {
                id: binding.id,
                reason,
            }
        })?;
        binding.process_scope.normalize().map_err(|reason| {
            RegistrationError::InvalidProcessScope {
                id: binding.id,
                reason,
            }
        })?;
        if !ids.insert(binding.id) {
            return Err(RegistrationError::DuplicateId { id: binding.id });
        }
        if binding.held.len() > 8 {
            return Err(RegistrationError::TooManyHeld { id: binding.id });
        }
        if binding.conditions.len() > 1 {
            return Err(RegistrationError::TooManyConditions { id: binding.id });
        }
        for condition in &binding.conditions {
            let TriggerCondition::PointerRegion(region) = condition;
            region
                .validate()
                .map_err(|reason| RegistrationError::InvalidPointerRegion {
                    id: binding.id,
                    reason,
                })?;
        }
        if let Activation::Drag {
            button,
            min_distance_px,
            max_start_ms,
        } = binding.activation
        {
            if !matches!(button, Button::Mouse(_)) {
                return Err(RegistrationError::InvalidDragButton {
                    id: binding.id,
                    button,
                });
            }
            if !(4..=500).contains(&min_distance_px) {
                return Err(RegistrationError::InvalidDragDistance {
                    id: binding.id,
                    min_distance_px,
                });
            }
            if !(20..=2000).contains(&max_start_ms) {
                return Err(RegistrationError::InvalidDragWindow {
                    id: binding.id,
                    max_start_ms,
                });
            }
        }
        for &button in binding.held.iter().chain(match &binding.activation {
            Activation::Press(button)
            | Activation::Hold { button, .. }
            | Activation::Drag { button, .. } => std::slice::from_ref(button),
            Activation::Wheel(_) => &[],
        }) {
            if matches!(button, Button::Key(key) if key == 0 || key > 254) {
                return Err(RegistrationError::InvalidButton {
                    id: binding.id,
                    button,
                });
            }
        }
        if let Activation::Hold { duration_ms, .. } = binding.activation
            && !(50..=10000).contains(&duration_ms)
        {
            return Err(RegistrationError::InvalidHoldDuration {
                id: binding.id,
                duration_ms,
            });
        }
        binding.held.sort_unstable();
        if binding.held.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(RegistrationError::DuplicateHeld { id: binding.id });
        }
        if let Some(button) = activation_button(binding.activation)
            && binding.held.contains(&button)
        {
            return Err(RegistrationError::TerminalInHeld { id: binding.id });
        }
    }
    for (index, left) in bindings.iter().enumerate() {
        for right in bindings.iter().skip(index + 1) {
            if left.process_scope.overlaps(&right.process_scope)
                && regions::conditions_overlap(&left.conditions, &right.conditions)
            {
                for (layer, other) in [(left, right), (right, left)] {
                    if let Some(button) = layer.keyboard.dedicated_prefix
                        && other.held.contains(&button)
                        && !other.keyboard.same_layer(&layer.keyboard)
                    {
                        return Err(RegistrationError::ConflictingPrefixPolicy {
                            first: left.id,
                            second: right.id,
                            button,
                        });
                    }
                }
            }
            let same_button = activation_button(left.activation)
                .is_some_and(|button| Some(button) == activation_button(right.activation));
            if left.held == right.held
                && (left.activation == right.activation || same_button)
                && regions::conditions_overlap(&left.conditions, &right.conditions)
                && left.process_scope.overlaps(&right.process_scope)
                && left.process_scope.priority() == right.process_scope.priority()
            {
                let timed = match (left.activation, right.activation) {
                    (
                        Activation::Hold { duration_ms, .. },
                        Activation::Drag { max_start_ms, .. },
                    ) => Some((left.id, right.id, duration_ms, max_start_ms)),
                    (
                        Activation::Drag { max_start_ms, .. },
                        Activation::Hold { duration_ms, .. },
                    ) => Some((right.id, left.id, duration_ms, max_start_ms)),
                    _ => None,
                };
                if let Some((hold_binding, drag_binding, hold_duration_ms, drag_max_start_ms)) =
                    timed
                {
                    if drag_max_start_ms >= hold_duration_ms {
                        return Err(RegistrationError::TimingConflict {
                            hold_binding,
                            drag_binding,
                            hold_duration_ms,
                            drag_max_start_ms,
                        });
                    }
                } else {
                    return Err(RegistrationError::DuplicateTrigger {
                        first: left.id,
                        second: right.id,
                    });
                }
            }
        }
        if let Some(button) = activation_button(left.activation)
            && let Some(prefix) = bindings.iter().find(|other| {
                other.held.contains(&button)
                    && regions::conditions_overlap(&left.conditions, &other.conditions)
                    && left.process_scope.overlaps(&other.process_scope)
            })
        {
            return Err(RegistrationError::TerminalIsPrefix {
                terminal_binding: left.id,
                prefix_binding: prefix.id,
            });
        }
    }
    Ok(bindings)
}

fn activation_button(activation: Activation) -> Option<Button> {
    match activation {
        Activation::Press(button)
        | Activation::Hold { button, .. }
        | Activation::Drag { button, .. } => Some(button),
        Activation::Wheel(_) => None,
    }
}
