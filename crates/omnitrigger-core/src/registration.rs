//! 注册限制、冲突检查与结构化错误。

use std::collections::BTreeSet;
use std::fmt;

use crate::{
    Activation, Button, ProcessScopeError, RegionError, TriggerCondition, TriggerSpec, WheelPolicy,
    regions,
};

/// Hook 匹配仍受固定上限约束；完整检查发生在注册时。
pub const MAX_BINDINGS: usize = 512;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistrationError {
    InvalidMouseCapturedLeader {
        id: u64,
    },
    MissingCapturedLeader {
        id: u64,
        button: Button,
    },
    InvalidMousePriority {
        id: u64,
        priority: u16,
    },
    InvalidMouseMatching {
        id: u64,
    },
    ConflictingModifierDelivery {
        first: u64,
        second: u64,
    },
    InvalidPointerProcessScope {
        id: u64,
        reason: ProcessScopeError,
    },
    InvalidWheelPolicy {
        id: u64,
    },
    InvalidRetainedPrefix {
        id: u64,
    },
    InvalidCapturedMatching {
        id: u64,
    },
    CapturedPriorityConflict {
        first: u64,
        second: u64,
    },
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
    InvalidGestureButton {
        id: u64,
        button: Button,
    },
    /// 同一按键组合在重叠范围内不能同时用于快速拖拽与手势：两者都以按下后的位移认领按压。
    GestureDragConflict {
        gesture_binding: u64,
        drag_binding: u64,
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
            Self::InvalidMouseCapturedLeader { .. } => "鼠标会话引用仅支持鼠标按下或滚轮，held 须为一个明确的非修饰键，不能同时建立捕获层、重复或单按恢复。",
            Self::MissingCapturedLeader { .. } => "鼠标规则缺少对应的键盘引导层，请先配置同一引导键的专用层或保留原功能层。",
            Self::InvalidMousePriority { .. } => "鼠标匹配顺序须为 0 至 511，数值越小越先匹配。",
            Self::InvalidMouseMatching { .. } => "鼠标观察模式仅支持鼠标按下或滚轮：精确模式 held 为空或一个修饰键，无控制键及快照模式 held 必须为空；不能同时配置键盘捕获层、重复、单按恢复、恢复超时或非全局单按范围。",
            Self::ConflictingModifierDelivery { .. } => "同一修饰键在重叠范围内同时用于鼠标观察和输入捕获，请分开其范围或统一交付策略。",
            Self::InvalidPointerProcessScope { reason, .. } => return write!(f, "指针窗口范围无效：{reason}"),
            Self::InvalidWheelPolicy { .. } => "逐事件滚轮策略仅支持滚轮触发。",
            Self::InvalidRetainedPrefix { .. } => "保留原功能前缀须为 held 中的明确键盘键，启用捕获匹配和单按恢复，与专用层互斥；时间须不超过 60000 毫秒。",
            Self::InvalidCapturedMatching { .. } => {
                "捕获集合需要显式键盘层（专用层或保留原功能层），最多一个附加键，顺序编号须小于 512；鼠标触发仅支持保留原功能层。"
            }
            Self::CapturedPriorityConflict { .. } => {
                "同一捕获层的重叠规则使用了相同顺序编号，请明确选择顺序。"
            }
            Self::InvalidDedicatedPrefix { .. } => {
                "专用前缀必须是当前组合中按住的明确键盘键，不能使用鼠标键或泛型修饰键。"
            }
            Self::InvalidKeyboardRepeat { .. } => {
                "键盘重复仅支持键盘按下触发，不支持鼠标、长按、拖拽或滚轮。"
            }
            Self::InvalidTapRestore { .. } => {
                "单按恢复需要显式键盘层；超时仅在启用恢复时可设置，应为 1 至 60000 毫秒。"
            }
            Self::InvalidTapProcessScope { reason, .. } => {
                return write!(f, "单按恢复的应用范围无效：{reason}");
            }
            Self::ConflictingPrefixPolicy { .. } => {
                "重叠应用和位置范围内，同一前缀的层模式（专用层或保留原功能层）、捕获范围、快速输入与单按恢复策略必须一致。"
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
            Self::InvalidGestureButton { .. } => "手势的目标必须是鼠标按键。",
            Self::GestureDragConflict { .. } => {
                "同一组合在重叠范围内不能同时用于快速拖拽和手势，请调整按键或应用范围。"
            }
            Self::TimingConflict { .. } => "同组合的快速拖拽窗口必须严格早于长按阈值。",
            Self::DuplicateId { .. } => "触发器编号重复，请重新添加该触发器。",
            Self::DuplicateHeld { .. } => "组合中的前缀按键重复，请重新录制。",
            Self::TerminalInHeld { .. } => "目标键不能同时作为当前组合的前缀，请调整组合后重试。",
            Self::DuplicateTrigger { .. } => "已存在相同组合，请编辑已有触发器。",
            Self::TerminalIsPrefix { .. } => {
                "目标键会先触发动作，无法继续作为另一组合的前缀，请调整组合或范围。"
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
        if binding.keyboard.matching.is_mouse_rule()
            && usize::from(binding.keyboard.matching.priority()) >= MAX_BINDINGS
        {
            return Err(RegistrationError::InvalidMousePriority {
                id: binding.id,
                priority: binding.keyboard.matching.priority(),
            });
        }
        if binding.keyboard.matching.is_mouse_captured_leader()
            && (binding.held.len() != 1
                || !binding.held.iter().all(|&button| {
                    matches!(button, Button::Key(1..=254)) && !crate::model::is_modifier(button)
                })
                || !matches!(
                    binding.activation,
                    Activation::Press(Button::Mouse(_)) | Activation::Wheel(_)
                )
                || binding.keyboard.prefix().is_some()
                || binding.keyboard.repeat
                || binding.keyboard.restore_on_tap
                || binding.keyboard.tap_timeout_ms.is_some()
                || !binding.keyboard.tap_scope.is_global())
        {
            return Err(RegistrationError::InvalidMouseCapturedLeader { id: binding.id });
        }
        if binding.keyboard.matching.is_mouse_exact()
            && (binding.held.len() > 1
                || binding
                    .held
                    .iter()
                    .any(|&button| !crate::model::is_modifier(button))
                || !matches!(
                    binding.activation,
                    Activation::Press(Button::Mouse(_)) | Activation::Wheel(_)
                )
                || binding.keyboard.prefix().is_some()
                || binding.keyboard.repeat
                || binding.keyboard.restore_on_tap
                || binding.keyboard.tap_timeout_ms.is_some()
                || !binding.keyboard.tap_scope.is_global())
        {
            return Err(RegistrationError::InvalidMouseMatching { id: binding.id });
        }
        if (binding.keyboard.matching.is_mouse_unmodified()
            || binding.keyboard.matching.is_mouse_sampled())
            && (!binding.held.is_empty()
                || !matches!(
                    binding.activation,
                    Activation::Press(Button::Mouse(_)) | Activation::Wheel(_)
                )
                || binding.keyboard.prefix().is_some()
                || binding.keyboard.repeat
                || binding.keyboard.restore_on_tap
                || binding.keyboard.tap_timeout_ms.is_some()
                || !binding.keyboard.tap_scope.is_global())
        {
            return Err(RegistrationError::InvalidMouseMatching { id: binding.id });
        }
        if binding.wheel != WheelPolicy::Notch
            && !matches!(binding.activation, Activation::Wheel(_))
        {
            return Err(RegistrationError::InvalidWheelPolicy { id: binding.id });
        }
        if let Some(policy) = binding.keyboard.retained_prefix.as_ref()
            && (binding.keyboard.dedicated_prefix.is_some()
                || !binding.keyboard.matching.is_captured()
                || !binding.keyboard.restore_on_tap
                || !matches!(policy.key, Button::Key(key) if (1..=254).contains(&key) && !matches!(key, 0x10..=0x12))
                || !binding.held.contains(&policy.key)
                || policy.quiet_ms > 60_000
                || policy.fast_ms > 60_000)
        {
            return Err(RegistrationError::InvalidRetainedPrefix { id: binding.id });
        }
        if let Some(policy) = binding.keyboard.retained_prefix.as_mut() {
            policy.capture_scope.normalize().map_err(|reason| {
                RegistrationError::InvalidProcessScope {
                    id: binding.id,
                    reason,
                }
            })?;
        }
        if binding.keyboard.matching.is_captured()
            && (binding.keyboard.prefix().is_none()
                || binding.held.len() > 2
                || binding
                    .held
                    .iter()
                    .any(|button| !matches!(button, Button::Key(_)))
                || !(matches!(binding.activation, Activation::Press(Button::Key(_)))
                    || (binding.keyboard.retained_prefix.is_some()
                        && matches!(
                            binding.activation,
                            Activation::Press(Button::Mouse(_)) | Activation::Wheel(_)
                        )))
                || usize::from(binding.keyboard.matching.priority()) >= MAX_BINDINGS)
        {
            return Err(RegistrationError::InvalidCapturedMatching { id: binding.id });
        }
        if let Some(button) = binding.keyboard.dedicated_prefix
            && (!matches!(button, Button::Key(key) if (1..=254).contains(&key) && !matches!(key, 0x10..=0x12))
                || !binding.held.contains(&button))
        {
            return Err(RegistrationError::InvalidDedicatedPrefix {
                id: binding.id,
                button,
            });
        }
        if binding.keyboard.repeat
            && !matches!(binding.activation, Activation::Press(Button::Key(_)))
        {
            return Err(RegistrationError::InvalidKeyboardRepeat { id: binding.id });
        }
        if ((binding.keyboard.restore_on_tap || binding.keyboard.tap_timeout_ms.is_some())
            && binding.keyboard.prefix().is_none())
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
        binding.pointer_scope.normalize().map_err(|reason| {
            RegistrationError::InvalidPointerProcessScope {
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
        if let Activation::Gesture { button, .. } = binding.activation
            && !matches!(button, Button::Mouse(_))
        {
            return Err(RegistrationError::InvalidGestureButton {
                id: binding.id,
                button,
            });
        }
        for &button in binding.held.iter().chain(match &binding.activation {
            Activation::Press(button)
            | Activation::Hold { button, .. }
            | Activation::Drag { button, .. }
            | Activation::Gesture { button, .. } => std::slice::from_ref(button),
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
    for binding in &bindings {
        if binding.keyboard.matching.is_mouse_captured_leader()
            && !bindings
                .iter()
                .any(|owner| owner.keyboard.prefix() == Some(binding.held[0]))
        {
            return Err(RegistrationError::MissingCapturedLeader {
                id: binding.id,
                button: binding.held[0],
            });
        }
    }
    for (index, left) in bindings.iter().enumerate() {
        for right in bindings.iter().skip(index + 1) {
            if left.scopes_overlap(right)
                && regions::conditions_overlap(&left.conditions, &right.conditions)
            {
                for (observed, captured) in [(left, right), (right, left)] {
                    if observed.keyboard.matching.is_mouse_exact()
                        && !captured.keyboard.matching.is_mouse_exact()
                        && captured
                            .held
                            .iter()
                            .copied()
                            .chain(activation_button(captured.activation))
                            .any(|button| observed.accepts_held_button(button))
                    {
                        return Err(RegistrationError::ConflictingModifierDelivery {
                            first: left.id,
                            second: right.id,
                        });
                    }
                }
            }
            for (retained, other) in [(left, right), (right, left)] {
                if let Some(policy) = retained.keyboard.retained_prefix.as_ref()
                    && other.keyboard.retained_prefix.is_none()
                    && !other.keyboard.matching.is_mouse_captured_leader()
                    && other.held.contains(&policy.key)
                    && policy.capture_scope.overlaps(&other.process_scope)
                    && regions::conditions_overlap(&retained.conditions, &other.conditions)
                    && !(other.keyboard.matching.is_captured()
                        && other.keyboard.prefix() != Some(policy.key))
                {
                    return Err(RegistrationError::ConflictingPrefixPolicy {
                        first: left.id,
                        second: right.id,
                        button: policy.key,
                    });
                }
            }
            if let (Some(a), Some(b)) = (
                &left.keyboard.retained_prefix,
                &right.keyboard.retained_prefix,
            ) && a.key == b.key
                && a.capture_scope.overlaps(&b.capture_scope)
                && regions::conditions_overlap(&left.conditions, &right.conditions)
                && !left.keyboard.same_layer(&right.keyboard)
            {
                return Err(RegistrationError::ConflictingPrefixPolicy {
                    first: left.id,
                    second: right.id,
                    button: a.key,
                });
            }
            if left.scopes_overlap(right)
                && regions::conditions_overlap(&left.conditions, &right.conditions)
            {
                for (layer, other) in [(left, right), (right, left)] {
                    if let Some(button) = layer.keyboard.prefix()
                        && other.held.contains(&button)
                        && !other.keyboard.matching.is_mouse_captured_leader()
                        && !other.keyboard.same_layer(&layer.keyboard)
                        && !(layer.keyboard.matching.is_captured()
                            && other.keyboard.matching.is_captured()
                            && other.keyboard.prefix() != Some(button))
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
            let captured_pair = left.keyboard.matching.is_captured()
                && right.keyboard.matching.is_captured()
                && left.keyboard.prefix() == right.keyboard.prefix();
            let ordered_mouse_pair = (left.keyboard.matching.is_mouse_rule()
                && right.keyboard.matching.is_mouse_rule())
                || left.keyboard.matching.is_mouse_unmodified()
                || right.keyboard.matching.is_mouse_unmodified()
                || left.keyboard.matching.is_mouse_sampled()
                || right.keyboard.matching.is_mouse_sampled();
            if captured_pair
                && same_button
                && left.keyboard.matching.priority() == right.keyboard.matching.priority()
                && left.scope_priority() == right.scope_priority()
                && left.scopes_overlap(right)
                && regions::conditions_overlap(&left.conditions, &right.conditions)
            {
                return Err(RegistrationError::CapturedPriorityConflict {
                    first: left.id,
                    second: right.id,
                });
            }
            // 同键的不同手势形状在松开时按相似度区分，相似到难以区分的视为重复；
            // 长按在位移超出容差时取消，与手势可共存。
            let gesture_pair = match (left.activation, right.activation) {
                (Activation::Gesture { shape: a, .. }, Activation::Gesture { shape: b, .. }) => {
                    a.similarity(&b) < crate::GESTURE_DUPLICATE
                }
                (Activation::Gesture { .. }, Activation::Hold { .. })
                | (Activation::Hold { .. }, Activation::Gesture { .. }) => true,
                _ => false,
            };
            if same_button
                && left.held_sets_overlap(right)
                && left.scopes_overlap(right)
                && regions::conditions_overlap(&left.conditions, &right.conditions)
            {
                let drag_gesture = match (left.activation, right.activation) {
                    (Activation::Gesture { .. }, Activation::Drag { .. }) => {
                        Some((left.id, right.id))
                    }
                    (Activation::Drag { .. }, Activation::Gesture { .. }) => {
                        Some((right.id, left.id))
                    }
                    _ => None,
                };
                if let Some((gesture_binding, drag_binding)) = drag_gesture {
                    return Err(RegistrationError::GestureDragConflict {
                        gesture_binding,
                        drag_binding,
                    });
                }
            }
            if left.held_sets_overlap(right)
                && !gesture_pair
                && (left.activation == right.activation || same_button)
                && regions::conditions_overlap(&left.conditions, &right.conditions)
                && left.scopes_overlap(right)
                && left.scope_priority() == right.scope_priority()
                && !((captured_pair || ordered_mouse_pair)
                    && left.keyboard.matching.priority() != right.keyboard.matching.priority())
                && !(left.keyboard.matching.is_captured()
                    && right.keyboard.matching.is_captured()
                    && left.keyboard.prefix() != right.keyboard.prefix())
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
                    && !sampled_terminal_preserves_plain_mouse_prefix(left, other, button)
                    // 两个显式捕获层由先按下的引导键决定归属，后一个键可以先作为末键或附加键。
                    && !(left.keyboard.matching.is_captured()
                        && other.keyboard.matching.is_captured())
                    && left.held.iter().all(|held| other.held.contains(held))
                    && regions::conditions_overlap(&left.conditions, &other.conditions)
                    && left.scopes_overlap(other)
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
        | Activation::Drag { button, .. }
        | Activation::Gesture { button, .. } => Some(button),
        Activation::Wheel(_) => None,
    }
}

/// 正向修饰条件可先触发，但全释放样本仍保留普通鼠标前缀入口。
/// 这不是Owned/Passed互斥：在台账为空而样本修饰键按住时，终键确实会抢先。
/// 仅放行正向修饰要求与无层的单鼠标前缀；无CSA及其他终键沿原保护拒绝。
fn sampled_terminal_preserves_plain_mouse_prefix(
    terminal: &TriggerSpec,
    prefix: &TriggerSpec,
    button: Button,
) -> bool {
    matches!(terminal.activation, Activation::Press(Button::Mouse(_)))
        && matches!(button, Button::Mouse(_))
        && matches!(terminal.keyboard.matching, crate::KeyboardMatching::MouseSampled { policy, .. }
            if policy.required != crate::ModifierRequirement::NoControlShiftAlt)
        && prefix.keyboard.matching == crate::KeyboardMatching::Exact
        && prefix.keyboard.prefix().is_none()
        && prefix.held.as_slice() == [button]
        && matches!(
            prefix.activation,
            Activation::Press(Button::Mouse(_)) | Activation::Wheel(_)
        )
}

#[cfg(test)]
mod mouse_unmodified_tests {
    use super::*;
    use crate::{
        KeyboardMatching, KeyboardPolicy, MouseButton, ProcessScope, RetainedPrefix, WheelDirection,
    };

    fn rule(id: u64, matching: KeyboardMatching, held: Vec<Button>) -> TriggerSpec {
        TriggerSpec {
            id,
            held,
            activation: Activation::Wheel(WheelDirection::Up),
            conditions: vec![],
            keyboard: KeyboardPolicy {
                matching,
                ..Default::default()
            },
            pointer_scope: Default::default(),
            process_scope: Default::default(),
            wheel: WheelPolicy::Packet,
        }
    }

    fn unmodified(id: u64, priority: u16) -> TriggerSpec {
        rule(
            id,
            KeyboardMatching::MouseUnmodified {
                priority,
                pointer_scope_priority: true,
            },
            vec![],
        )
    }

    #[test]
    fn unmodified_registration_rejects_every_capture_timing_and_held_option() {
        let base = unmodified(1, 0);
        let key = Button::Key(0x51);
        let mut invalid = Vec::new();
        let mut binding = base.clone();
        binding.held = vec![key];
        invalid.push(binding);
        let mut binding = base.clone();
        binding.activation = Activation::Press(key);
        invalid.push(binding);
        let mut binding = base.clone();
        binding.activation = Activation::Hold {
            button: Button::Mouse(MouseButton::Right),
            duration_ms: 100,
        };
        binding.wheel = WheelPolicy::Notch;
        invalid.push(binding);
        let mut binding = base.clone();
        binding.activation = Activation::Drag {
            button: Button::Mouse(MouseButton::Right),
            min_distance_px: 10,
            max_start_ms: 100,
        };
        binding.wheel = WheelPolicy::Notch;
        invalid.push(binding);
        let mut binding = base.clone();
        binding.keyboard.dedicated_prefix = Some(key);
        invalid.push(binding);
        let mut binding = base.clone();
        binding.keyboard.retained_prefix = Some(Box::new(RetainedPrefix {
            key,
            capture_scope: Default::default(),
            quiet_ms: 0,
            fast_ms: 0,
            block_with_primary_mouse: false,
        }));
        invalid.push(binding);
        let mut binding = base.clone();
        binding.keyboard.repeat = true;
        invalid.push(binding);
        let mut binding = base.clone();
        binding.keyboard.restore_on_tap = true;
        invalid.push(binding);
        let mut binding = base.clone();
        binding.keyboard.tap_timeout_ms = Some(10);
        invalid.push(binding);
        let mut binding = base;
        binding.keyboard.tap_scope.allow = vec!["sample".into()];
        invalid.push(binding);
        for binding in invalid {
            assert_eq!(
                validate(vec![binding]).unwrap_err(),
                RegistrationError::InvalidMouseMatching { id: 1 }
            );
        }
        assert!(validate(vec![unmodified(1, 511)]).is_ok());
        assert_eq!(
            validate(vec![unmodified(1, 512)]).unwrap_err(),
            RegistrationError::InvalidMousePriority {
                id: 1,
                priority: 512
            }
        );
    }

    #[test]
    fn unmodified_overlap_distinguishes_owned_exact_from_passed_observation_at_equal_rank() {
        for matching in [
            KeyboardMatching::Exact,
            KeyboardMatching::MouseExact {
                priority: 0,
                pointer_scope_priority: true,
            },
            KeyboardMatching::MouseUnmodified {
                priority: 0,
                pointer_scope_priority: true,
            },
        ] {
            let empty = rule(2, matching, vec![]);
            assert_eq!(
                validate(vec![unmodified(1, 0), empty.clone()]).unwrap_err(),
                RegistrationError::DuplicateTrigger {
                    first: 1,
                    second: 2
                }
            );
            assert_eq!(
                validate(vec![empty.clone(), unmodified(1, 0)]).unwrap_err(),
                RegistrationError::DuplicateTrigger {
                    first: 2,
                    second: 1
                }
            );
            assert!(validate(vec![unmodified(1, 1), empty]).is_ok());
        }
        for held in [
            vec![Button::Key(0x51)],
            vec![Button::Key(0x5b)],
            vec![Button::Mouse(MouseButton::Left)],
            vec![Button::Mouse(MouseButton::X1)],
            vec![Button::Key(0x51), Button::Key(0x5c)],
        ] {
            let exact = rule(2, KeyboardMatching::Exact, held);
            assert!(validate(vec![unmodified(1, 0), exact.clone()]).is_ok());
            assert!(validate(vec![exact.clone(), unmodified(1, 0)]).is_ok());
            assert!(validate(vec![unmodified(1, 1), exact]).is_ok());
        }
        let strict_win = rule(
            2,
            KeyboardMatching::MouseExact {
                priority: 2,
                pointer_scope_priority: true,
            },
            vec![Button::Key(0x5b)],
        );
        assert_eq!(
            validate(vec![unmodified(1, 2), strict_win.clone()]).unwrap_err(),
            RegistrationError::DuplicateTrigger {
                first: 1,
                second: 2
            }
        );
        assert!(validate(vec![unmodified(1, 1), strict_win]).is_ok());
    }

    #[test]
    fn control_shift_alt_requirements_are_disjoint_but_scope_priority_still_orders_real_overlap() {
        for key in [0x10, 0x11, 0x12, 0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5] {
            let other = rule(
                2,
                KeyboardMatching::MouseExact {
                    priority: 0,
                    pointer_scope_priority: true,
                },
                vec![Button::Key(key)],
            );
            assert!(validate(vec![unmodified(1, 0), other]).is_ok());
            let exact = rule(
                2,
                KeyboardMatching::Exact,
                vec![Button::Key(0x51), Button::Key(key)],
            );
            assert!(validate(vec![unmodified(1, 0), exact]).is_ok());
        }
        let mut scoped = unmodified(2, 0);
        scoped.pointer_scope = ProcessScope {
            allow: vec!["sample".into()],
            ..Default::default()
        };
        assert!(validate(vec![unmodified(1, 0), scoped.clone()]).is_ok());
        scoped.keyboard.matching = KeyboardMatching::MouseUnmodified {
            priority: 0,
            pointer_scope_priority: false,
        };
        assert_eq!(
            validate(vec![unmodified(1, 0), scoped]).unwrap_err(),
            RegistrationError::DuplicateTrigger {
                first: 1,
                second: 2
            }
        );
    }

    #[test]
    fn captured_mouse_leader_and_retained_capture_are_ownership_disjoint_at_equal_rank() {
        let key = Button::Key(0x51);
        let captured = rule(
            2,
            KeyboardMatching::MouseCapturedLeader {
                priority: 0,
                pointer_scope_priority: true,
            },
            vec![key],
        );
        let mut owner = rule(3, KeyboardMatching::Captured { priority: 0 }, vec![key]);
        owner.activation = Activation::Press(Button::Key(0x4a));
        owner.wheel = WheelPolicy::Notch;
        owner.keyboard.dedicated_prefix = Some(key);
        assert!(validate(vec![unmodified(1, 0), captured.clone(), owner.clone()]).is_ok());
        assert!(validate(vec![captured.clone(), unmodified(1, 0), owner]).is_ok());
        let mut retained = rule(3, KeyboardMatching::Captured { priority: 0 }, vec![key]);
        retained.keyboard.retained_prefix = Some(Box::new(RetainedPrefix {
            key,
            capture_scope: Default::default(),
            quiet_ms: 0,
            fast_ms: 0,
            block_with_primary_mouse: false,
        }));
        retained.keyboard.restore_on_tap = true;
        assert!(validate(vec![unmodified(1, 0), retained.clone()]).is_ok());
        assert!(validate(vec![retained, unmodified(1, 0)]).is_ok());
    }
}

#[cfg(test)]
mod mouse_sampled_tests {
    use super::*;
    use crate::{
        KeyboardMatching, KeyboardPolicy, ModifierRequirement, MouseModifierPolicy, WheelDirection,
    };

    fn rule(id: u64, required: ModifierRequirement) -> TriggerSpec {
        TriggerSpec {
            id,
            held: vec![],
            activation: Activation::Wheel(WheelDirection::Up),
            conditions: vec![],
            keyboard: KeyboardPolicy {
                matching: KeyboardMatching::MouseSampled {
                    policy: MouseModifierPolicy {
                        required,
                        allow_other_modifiers: required == ModifierRequirement::NoControlShiftAlt,
                        allow_extra_passed_keys: true,
                        allow_extra_passed_mouse: false,
                    },
                    priority: 0,
                    pointer_scope_priority: true,
                },
                ..Default::default()
            },
            process_scope: Default::default(),
            pointer_scope: Default::default(),
            wheel: WheelPolicy::Packet,
        }
    }

    #[test]
    fn sampled_modifier_domains_keep_true_conflicts_and_owned_exact_disjointness() {
        let control = rule(1, ModifierRequirement::Control);
        assert!(
            validate(vec![
                control.clone(),
                rule(2, ModifierRequirement::NoControlShiftAlt)
            ])
            .is_ok()
        );
        assert!(validate(vec![control.clone(), rule(2, ModifierRequirement::LeftAlt)]).is_ok());
        assert_eq!(
            validate(vec![
                control.clone(),
                rule(2, ModifierRequirement::LeftControl)
            ])
            .unwrap_err(),
            RegistrationError::DuplicateTrigger {
                first: 1,
                second: 2
            }
        );
        for matching in [
            KeyboardMatching::Exact,
            KeyboardMatching::MouseExact {
                priority: 0,
                pointer_scope_priority: true,
            },
            KeyboardMatching::MouseUnmodified {
                priority: 0,
                pointer_scope_priority: true,
            },
        ] {
            let mut other = control.clone();
            other.id = 2;
            other.keyboard.matching = matching;
            assert_eq!(
                validate(vec![control.clone(), other]).unwrap_err(),
                RegistrationError::DuplicateTrigger {
                    first: 1,
                    second: 2
                }
            );
        }
        let mut owned = control.clone();
        owned.id = 2;
        owned.keyboard.matching = KeyboardMatching::Exact;
        owned.held = vec![Button::Mouse(crate::MouseButton::X1)];
        assert!(validate(vec![control, owned]).is_ok());
    }

    #[test]
    fn sampled_shape_and_priority_reject_invalid_fields_without_normalizing_held() {
        let mut held = rule(1, ModifierRequirement::Control);
        held.held.push(Button::Key(0x11));
        assert_eq!(
            validate(vec![held]).unwrap_err(),
            RegistrationError::InvalidMouseMatching { id: 1 }
        );
        let mut repeated = rule(1, ModifierRequirement::Control);
        repeated.keyboard.repeat = true;
        assert_eq!(
            validate(vec![repeated]).unwrap_err(),
            RegistrationError::InvalidMouseMatching { id: 1 }
        );
        let mut high = rule(1, ModifierRequirement::Control);
        if let KeyboardMatching::MouseSampled { priority, .. } = &mut high.keyboard.matching {
            *priority = 512;
        }
        assert_eq!(
            validate(vec![high]).unwrap_err(),
            RegistrationError::InvalidMousePriority {
                id: 1,
                priority: 512
            }
        );
    }

    #[test]
    fn allowing_other_snapshot_modifiers_creates_a_real_control_alt_conflict() {
        let mut control = rule(1, ModifierRequirement::Control);
        let mut alt = rule(2, ModifierRequirement::LeftAlt);
        for binding in [&mut control, &mut alt] {
            if let KeyboardMatching::MouseSampled { policy, .. } = &mut binding.keyboard.matching {
                policy.allow_other_modifiers = true;
            }
        }
        assert_eq!(
            validate(vec![control.clone(), alt.clone()]).unwrap_err(),
            RegistrationError::DuplicateTrigger {
                first: 1,
                second: 2
            }
        );
        if let KeyboardMatching::MouseSampled { priority, .. } = &mut alt.keyboard.matching {
            *priority = 1;
        }
        assert!(validate(vec![control, alt]).is_ok());
    }
}
