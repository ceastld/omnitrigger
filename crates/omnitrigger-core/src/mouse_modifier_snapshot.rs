//! 逐事件的鼠标修饰键快照校验；不读取系统键状态、时钟或改变输入所有权。
//! 本模块需调用方显式接线。快照不能证明某个按压来自物理设备或目标程序。

use crate::{Button, InputKind, KeyboardMatching, MouseButton, TriggerSpec};

const MAX_PRESSES: usize = 259;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModifierSides {
    pub left_shift: bool,
    pub right_shift: bool,
    pub left_control: bool,
    pub right_control: bool,
    pub left_alt: bool,
    pub right_alt: bool,
    pub left_windows: bool,
    pub right_windows: bool,
}

impl ModifierSides {
    fn from_bits(bits: u8) -> Self {
        Self {
            left_shift: bits & 1 != 0,
            right_shift: bits & 2 != 0,
            left_control: bits & 4 != 0,
            right_control: bits & 8 != 0,
            left_alt: bits & 16 != 0,
            right_alt: bits & 32 != 0,
            left_windows: bits & 64 != 0,
            right_windows: bits & 128 != 0,
        }
    }

    fn keys(self) -> [(u16, bool); 8] {
        [
            (0xa0, self.left_shift),
            (0xa1, self.right_shift),
            (0xa2, self.left_control),
            (0xa3, self.right_control),
            (0xa4, self.left_alt),
            (0xa5, self.right_alt),
            (0x5b, self.left_windows),
            (0x5c, self.right_windows),
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModifierRequirement {
    /// 显式快照无Ctrl/Shift/Alt；Win是否允许仍由allow_other_modifiers指定。
    NoControlShiftAlt,
    Shift,
    Control,
    Alt,
    LeftShift,
    RightShift,
    LeftControl,
    RightControl,
    LeftAlt,
    RightAlt,
    LeftWindows,
    RightWindows,
}

impl ModifierRequirement {
    fn accepts_key(self, key: u16) -> bool {
        match self {
            Self::NoControlShiftAlt => false,
            Self::Shift => matches!(key, 0x10 | 0xa0 | 0xa1),
            Self::Control => matches!(key, 0x11 | 0xa2 | 0xa3),
            Self::Alt => matches!(key, 0x12 | 0xa4 | 0xa5),
            Self::LeftShift => key == 0xa0,
            Self::RightShift => key == 0xa1,
            Self::LeftControl => key == 0xa2,
            Self::RightControl => key == 0xa3,
            Self::LeftAlt => key == 0xa4,
            Self::RightAlt => key == 0xa5,
            Self::LeftWindows => key == 0x5b,
            Self::RightWindows => key == 0x5c,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseModifierPolicy {
    pub required: ModifierRequirement,
    /// false只接受所需修饰键；true接受其他快照修饰键。必须显式选择。
    pub allow_other_modifiers: bool,
    pub allow_extra_passed_keys: bool,
    pub allow_extra_passed_mouse: bool,
}

impl MouseModifierPolicy {
    fn accepts_modifiers(self, modifiers: ModifierSides) -> bool {
        let required = if self.required == ModifierRequirement::NoControlShiftAlt {
            !modifiers
                .keys()
                .iter()
                .any(|&(key, down)| down && matches!(key, 0xa0..=0xa5))
        } else {
            modifiers
                .keys()
                .iter()
                .any(|&(key, down)| down && self.required.accepts_key(key))
        };
        required
            && (self.allow_other_modifiers
                || !modifiers
                    .keys()
                    .iter()
                    .any(|&(key, down)| down && !self.required.accepts_key(key)))
    }

    pub(crate) fn accepts_passed_key(self, key: u16) -> bool {
        if self.required == ModifierRequirement::NoControlShiftAlt
            && matches!(key, 0x10..=0x12 | 0xa0..=0xa5)
        {
            return false;
        }
        self.required.accepts_key(key)
            || (is_modifier_key(key) && self.allow_other_modifiers)
            || (!is_modifier_key(key) && self.allow_extra_passed_keys)
    }

    /// 仅比较修饰状态域；注册时还须同时比较activation、范围、区域和显式rank。
    /// 空核心按压是共同见证，因此额外Passed输入开关不改变该状态域交集。
    pub fn modifier_states_overlap(self, other: Self) -> bool {
        (0..=u8::MAX).any(|bits| {
            let modifiers = ModifierSides::from_bits(bits);
            self.accepts_modifiers(modifiers) && other.accepts_modifiers(modifiers)
        })
    }

    /// 对已经通过注册校验的既有TriggerSpec比较held所有权域，不选择赢家或改变绑定。
    /// 快照修饰键可在核心pressed为空时成立，不能把无控制键模式误判为必然互斥。
    pub fn overlaps_existing_held(self, other: &TriggerSpec) -> bool {
        match other.keyboard.matching {
            KeyboardMatching::MouseSampled { policy, .. } => self.modifier_states_overlap(policy),
            KeyboardMatching::Exact => other.held.is_empty(),
            KeyboardMatching::Captured { .. } | KeyboardMatching::MouseCapturedLeader { .. } => {
                false
            }
            KeyboardMatching::MouseUnmodified { .. } => true,
            KeyboardMatching::MouseExact { .. } => other.held.iter().all(|&required| {
                (1..=254).any(|key| {
                    let physical = Button::Key(key);
                    (required == physical || other.accepts_held_button(physical))
                        && self.accepts_passed_key(key)
                })
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseContextEvent {
    pub stream_id: u64,
    pub serial: u64,
    pub epoch: u64,
    pub observed_at_ms: u64,
    pub kind: InputKind,
    pub injected: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseModifierSnapshot {
    pub stream_id: u64,
    pub event_serial: u64,
    pub epoch: u64,
    pub sampled_at_ms: u64,
    pub modifiers: ModifierSides,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseModifierInput {
    pub stream_id: u64,
    pub serial: u64,
    pub epoch: u64,
    pub snapshot: Option<MouseModifierSnapshot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservationOwnership {
    Passed,
    /// 包括等待、消费、观察前缀和取消后的释放收尾，不可借给观察规则。
    Owned,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObservedPress {
    pub button: Button,
    pub ownership: ObservationOwnership,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseCaptureOwnership {
    pub keyboard_layer: bool,
    pub terminal_press: bool,
    pub mouse_session: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseSnapshotError {
    InvalidAgeLimit,
    InvalidStream,
    StaleStream,
    Closed,
    EpochExhausted,
    NotMouseCandidate,
    Injected,
    InvalidSerial,
    StaleEvent,
    EventEpochMismatch,
    EventStreamMismatch,
    ClockWentBackward,
    MissingSnapshot,
    SnapshotIdentityMismatch,
    FutureSnapshot,
    ExpiredSnapshot,
    InvalidPressSet,
    CaptureOwned,
    PressOwned,
    ContextNoLongerCurrent,
}

/// 只在当前事件内使用；每次匹配仍由gate核对代次及事件标识，取消后不能重用。
#[derive(Clone, Copy, Debug)]
pub struct MouseModifierContext {
    event: MouseContextEvent,
    modifiers: ModifierSides,
    /// 固定大小的Passed快照，不借用Engine或外层数组，避免接线时自引用/长期借用。
    presses: [u64; 5],
}

pub struct MouseModifierGate {
    stream_id: u64,
    epoch: u64,
    last_serial: u64,
    last_time_ms: Option<u64>,
    max_snapshot_age_ms: u32,
    closed: bool,
}

impl MouseModifierGate {
    /// 接受0–1000ms的显式上限；0表示只接受与事件同一毫秒的样本。
    /// stream_id由调用方分配，须区分并存或重建后的事件流，不作为设备身份。
    pub fn new(stream_id: u64, max_snapshot_age_ms: u32) -> Result<Self, MouseSnapshotError> {
        if stream_id == 0 {
            return Err(MouseSnapshotError::InvalidStream);
        }
        if max_snapshot_age_ms > 1000 {
            return Err(MouseSnapshotError::InvalidAgeLimit);
        }
        Ok(Self {
            stream_id,
            epoch: 1,
            last_serial: 0,
            last_time_ms: None,
            max_snapshot_age_ms,
            closed: false,
        })
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn stream_id(&self) -> u64 {
        self.stream_id
    }

    /// 外层须在取消、配置替换、暂停或输入恢复边界调用。不清除或伪造按键状态。
    pub fn cancel(&mut self) -> Result<u64, MouseSnapshotError> {
        match self.epoch.checked_add(1) {
            Some(epoch) if !self.closed => {
                self.epoch = epoch;
                Ok(epoch)
            }
            _ => {
                self.closed = true;
                Err(MouseSnapshotError::EpochExhausted)
            }
        }
    }

    pub fn observe(
        &mut self,
        event: MouseContextEvent,
        snapshot: Option<MouseModifierSnapshot>,
        presses: &[ObservedPress],
        ownership: MouseCaptureOwnership,
    ) -> Result<MouseModifierContext, MouseSnapshotError> {
        let mut context = self.observe_sample(event, snapshot)?;
        context.presses = pack_passed_presses(presses, ownership)?;
        Ok(context)
    }

    /// Engine内部先核验事件样本，再在统一候选选择时检查当时实际输入所有权。
    pub(crate) fn observe_sample(
        &mut self,
        event: MouseContextEvent,
        snapshot: Option<MouseModifierSnapshot>,
    ) -> Result<MouseModifierContext, MouseSnapshotError> {
        if self.closed {
            return Err(MouseSnapshotError::Closed);
        }
        if event.injected {
            return Err(MouseSnapshotError::Injected);
        }
        let candidate = match event.kind {
            InputKind::Down(Button::Mouse(_)) => true,
            InputKind::Wheel { delta, .. } => delta != 0,
            _ => false,
        };
        if !candidate {
            return Err(MouseSnapshotError::NotMouseCandidate);
        }
        if event.serial == 0 {
            return Err(MouseSnapshotError::InvalidSerial);
        }
        if event.stream_id != self.stream_id {
            return Err(MouseSnapshotError::EventStreamMismatch);
        }
        if event.epoch != self.epoch {
            return Err(MouseSnapshotError::EventEpochMismatch);
        }
        if event.serial <= self.last_serial {
            return Err(MouseSnapshotError::StaleEvent);
        }
        // 一次物理事件只观察一次；失败后不能拿更新的快照重试同一事件。
        self.last_serial = event.serial;
        if self
            .last_time_ms
            .is_some_and(|last| event.observed_at_ms < last)
        {
            return Err(MouseSnapshotError::ClockWentBackward);
        }
        self.last_time_ms = Some(event.observed_at_ms);
        let snapshot = snapshot.ok_or(MouseSnapshotError::MissingSnapshot)?;
        if snapshot.stream_id != event.stream_id
            || snapshot.event_serial != event.serial
            || snapshot.epoch != event.epoch
        {
            return Err(MouseSnapshotError::SnapshotIdentityMismatch);
        }
        let age = event
            .observed_at_ms
            .checked_sub(snapshot.sampled_at_ms)
            .ok_or(MouseSnapshotError::FutureSnapshot)?;
        if age > u64::from(self.max_snapshot_age_ms) {
            return Err(MouseSnapshotError::ExpiredSnapshot);
        }
        Ok(MouseModifierContext {
            event,
            modifiers: snapshot.modifiers,
            presses: [0; 5],
        })
    }

    pub fn matches(
        &self,
        context: &MouseModifierContext,
        policy: MouseModifierPolicy,
    ) -> Result<bool, MouseSnapshotError> {
        if self.closed
            || context.event.stream_id != self.stream_id
            || context.event.epoch != self.epoch
            || context.event.serial != self.last_serial
        {
            return Err(MouseSnapshotError::ContextNoLongerCurrent);
        }
        if !policy.accepts_modifiers(context.modifiers) {
            return Ok(false);
        }
        Ok(context
            .presses
            .iter()
            .enumerate()
            .all(|(word_index, &word)| {
                let mut remaining = word;
                while remaining != 0 {
                    let index = word_index * 64 + remaining.trailing_zeros() as usize;
                    let accepted = if index < 254 {
                        policy.accepts_passed_key(index as u16 + 1)
                    } else {
                        policy.allow_extra_passed_mouse
                    };
                    if !accepted {
                        return false;
                    }
                    remaining &= remaining - 1;
                }
                true
            }))
    }
}

fn pack_passed_presses(
    presses: &[ObservedPress],
    ownership: MouseCaptureOwnership,
) -> Result<[u64; 5], MouseSnapshotError> {
    if ownership.keyboard_layer || ownership.terminal_press || ownership.mouse_session {
        return Err(MouseSnapshotError::CaptureOwned);
    }
    if presses.len() > MAX_PRESSES {
        return Err(MouseSnapshotError::InvalidPressSet);
    }
    let mut seen = [0u64; 5];
    for press in presses {
        let index = press_index(press.button).ok_or(MouseSnapshotError::InvalidPressSet)?;
        let bit = 1u64 << (index % 64);
        if seen[index / 64] & bit != 0 {
            return Err(MouseSnapshotError::InvalidPressSet);
        }
        seen[index / 64] |= bit;
        if press.ownership != ObservationOwnership::Passed {
            return Err(MouseSnapshotError::PressOwned);
        }
    }
    Ok(seen)
}

fn is_modifier_key(key: u16) -> bool {
    matches!(key, 0x10..=0x12 | 0xa0..=0xa5 | 0x5b..=0x5c)
}

fn press_index(button: Button) -> Option<usize> {
    match button {
        Button::Key(key @ 1..=254) => Some(usize::from(key - 1)),
        Button::Mouse(mouse) => Some(
            254 + match mouse {
                MouseButton::Left => 0,
                MouseButton::Right => 1,
                MouseButton::Middle => 2,
                MouseButton::X1 => 3,
                MouseButton::X2 => 4,
            },
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(serial: u64, epoch: u64, time: u64) -> MouseContextEvent {
        MouseContextEvent {
            stream_id: 1,
            serial,
            epoch,
            observed_at_ms: time,
            kind: InputKind::Down(Button::Mouse(MouseButton::X1)),
            injected: false,
        }
    }
    fn snapshot(event: MouseContextEvent) -> MouseModifierSnapshot {
        MouseModifierSnapshot {
            stream_id: event.stream_id,
            event_serial: event.serial,
            epoch: event.epoch,
            sampled_at_ms: event.observed_at_ms,
            modifiers: ModifierSides {
                left_control: true,
                ..Default::default()
            },
        }
    }
    fn policy() -> MouseModifierPolicy {
        MouseModifierPolicy {
            required: ModifierRequirement::Control,
            allow_other_modifiers: false,
            allow_extra_passed_keys: true,
            allow_extra_passed_mouse: false,
        }
    }

    #[test]
    fn family_and_sides_are_explicit_without_creating_missing_key_ownership() {
        for required in [
            ModifierRequirement::Control,
            ModifierRequirement::LeftControl,
        ] {
            let mut gate = MouseModifierGate::new(1, 0).unwrap();
            let e = event(1, gate.epoch(), 100);
            let context = gate
                .observe(e, Some(snapshot(e)), &[], MouseCaptureOwnership::default())
                .unwrap();
            assert!(
                gate.matches(
                    &context,
                    MouseModifierPolicy {
                        required,
                        ..policy()
                    }
                )
                .unwrap()
            );
            assert!(
                !gate
                    .matches(
                        &context,
                        MouseModifierPolicy {
                            required: ModifierRequirement::RightControl,
                            ..policy()
                        }
                    )
                    .unwrap()
            );
        }
    }

    #[test]
    fn passed_extras_and_other_modifiers_require_independent_explicit_options() {
        let mut gate = MouseModifierGate::new(1, 10).unwrap();
        let e = event(1, 1, 100);
        let extras = [
            ObservedPress {
                button: Button::Key(0x51),
                ownership: ObservationOwnership::Passed,
            },
            ObservedPress {
                button: Button::Mouse(MouseButton::X2),
                ownership: ObservationOwnership::Passed,
            },
        ];
        let mut sample = snapshot(e);
        sample.modifiers.left_alt = true;
        let context = gate
            .observe(e, Some(sample), &extras, MouseCaptureOwnership::default())
            .unwrap();
        assert!(!gate.matches(&context, policy()).unwrap());
        assert!(
            !gate
                .matches(
                    &context,
                    MouseModifierPolicy {
                        allow_other_modifiers: true,
                        ..policy()
                    }
                )
                .unwrap()
        );
        assert!(
            gate.matches(
                &context,
                MouseModifierPolicy {
                    allow_other_modifiers: true,
                    allow_extra_passed_mouse: true,
                    ..policy()
                }
            )
            .unwrap()
        );
        assert!(
            !gate
                .matches(
                    &context,
                    MouseModifierPolicy {
                        allow_other_modifiers: true,
                        allow_extra_passed_mouse: true,
                        allow_extra_passed_keys: false,
                        ..policy()
                    }
                )
                .unwrap()
        );
    }

    #[test]
    fn missing_stale_future_and_wrong_identity_samples_fail_closed_and_cannot_retry_event() {
        for mode in 0..5 {
            let mut gate = MouseModifierGate::new(1, 5).unwrap();
            let e = event(1, 1, 100);
            let mut sample = snapshot(e);
            let expected = match mode {
                0 => MouseSnapshotError::MissingSnapshot,
                1 => {
                    sample.event_serial = 2;
                    MouseSnapshotError::SnapshotIdentityMismatch
                }
                2 => {
                    sample.epoch = 2;
                    MouseSnapshotError::SnapshotIdentityMismatch
                }
                3 => {
                    sample.sampled_at_ms = 101;
                    MouseSnapshotError::FutureSnapshot
                }
                _ => {
                    sample.sampled_at_ms = 94;
                    MouseSnapshotError::ExpiredSnapshot
                }
            };
            assert_eq!(
                gate.observe(
                    e,
                    if mode == 0 { None } else { Some(sample) },
                    &[],
                    MouseCaptureOwnership::default()
                )
                .unwrap_err(),
                expected
            );
            assert_eq!(
                gate.observe(e, Some(snapshot(e)), &[], MouseCaptureOwnership::default())
                    .unwrap_err(),
                MouseSnapshotError::StaleEvent
            );
        }
    }

    #[test]
    fn ownership_rejection_includes_keyboard_mouse_terminal_and_capture_sessions() {
        for button in [
            Button::Key(0xa2),
            Button::Key(0x51),
            Button::Mouse(MouseButton::Left),
        ] {
            let mut gate = MouseModifierGate::new(1, 0).unwrap();
            let e = event(1, 1, 0);
            let presses = [ObservedPress {
                button,
                ownership: ObservationOwnership::Owned,
            }];
            assert_eq!(
                gate.observe(
                    e,
                    Some(snapshot(e)),
                    &presses,
                    MouseCaptureOwnership::default()
                )
                .unwrap_err(),
                MouseSnapshotError::PressOwned
            );
        }
        for ownership in [
            MouseCaptureOwnership {
                keyboard_layer: true,
                ..Default::default()
            },
            MouseCaptureOwnership {
                terminal_press: true,
                ..Default::default()
            },
            MouseCaptureOwnership {
                mouse_session: true,
                ..Default::default()
            },
        ] {
            let mut gate = MouseModifierGate::new(1, 0).unwrap();
            let e = event(1, 1, 0);
            assert_eq!(
                gate.observe(e, Some(snapshot(e)), &[], ownership)
                    .unwrap_err(),
                MouseSnapshotError::CaptureOwned
            );
        }
    }

    #[test]
    fn cancel_and_new_events_revoke_previously_valid_context_without_mutating_presses() {
        let mut gate = MouseModifierGate::new(1, 0).unwrap();
        let e = event(1, 1, 100);
        let context = gate
            .observe(e, Some(snapshot(e)), &[], MouseCaptureOwnership::default())
            .unwrap();
        assert_eq!(gate.cancel().unwrap(), 2);
        assert_eq!(
            gate.matches(&context, policy()).unwrap_err(),
            MouseSnapshotError::ContextNoLongerCurrent
        );
        assert_eq!(
            gate.observe(
                event(2, 1, 101),
                None,
                &[],
                MouseCaptureOwnership::default()
            )
            .unwrap_err(),
            MouseSnapshotError::EventEpochMismatch
        );
        let next = event(2, 2, 101);
        let current = gate
            .observe(
                next,
                Some(snapshot(next)),
                &[],
                MouseCaptureOwnership::default(),
            )
            .unwrap();
        let failed = event(3, 2, 102);
        assert_eq!(
            gate.observe(failed, None, &[], MouseCaptureOwnership::default())
                .unwrap_err(),
            MouseSnapshotError::MissingSnapshot
        );
        assert_eq!(
            gate.matches(&current, policy()).unwrap_err(),
            MouseSnapshotError::ContextNoLongerCurrent
        );
    }

    #[test]
    fn injected_releases_and_zero_wheel_never_advance_observation_state() {
        let mut gate = MouseModifierGate::new(1, 0).unwrap();
        for kind in [
            InputKind::Up(Button::Mouse(MouseButton::X1)),
            InputKind::Down(Button::Key(65)),
            InputKind::Wheel {
                axis: crate::Axis::Vertical,
                delta: 0,
            },
        ] {
            assert_eq!(
                gate.observe(
                    MouseContextEvent {
                        kind,
                        ..event(1, 1, 100)
                    },
                    None,
                    &[],
                    MouseCaptureOwnership::default()
                )
                .unwrap_err(),
                MouseSnapshotError::NotMouseCandidate
            );
        }
        assert_eq!(
            gate.observe(
                MouseContextEvent {
                    injected: true,
                    ..event(1, 1, 200)
                },
                None,
                &[],
                MouseCaptureOwnership::default()
            )
            .unwrap_err(),
            MouseSnapshotError::Injected
        );
        let e = event(1, 1, 100);
        assert!(
            gate.observe(e, Some(snapshot(e)), &[], MouseCaptureOwnership::default())
                .is_ok()
        );
    }

    #[test]
    fn age_boundary_clock_reversal_and_epoch_exhaustion_are_explicit() {
        assert!(matches!(
            MouseModifierGate::new(1, 1001),
            Err(MouseSnapshotError::InvalidAgeLimit)
        ));
        let mut gate = MouseModifierGate::new(1, 5).unwrap();
        let e = event(1, 1, 100);
        let mut sample = snapshot(e);
        sample.sampled_at_ms = 95;
        assert!(
            gate.observe(e, Some(sample), &[], MouseCaptureOwnership::default())
                .is_ok()
        );
        assert_eq!(
            gate.observe(event(2, 1, 99), None, &[], MouseCaptureOwnership::default())
                .unwrap_err(),
            MouseSnapshotError::ClockWentBackward
        );
        assert_eq!(
            gate.observe(
                event(2, 1, 100),
                None,
                &[],
                MouseCaptureOwnership::default()
            )
            .unwrap_err(),
            MouseSnapshotError::StaleEvent
        );
        gate.epoch = u64::MAX;
        assert_eq!(
            gate.cancel().unwrap_err(),
            MouseSnapshotError::EpochExhausted
        );
        assert_eq!(
            gate.observe(
                event(3, u64::MAX, 101),
                None,
                &[],
                MouseCaptureOwnership::default()
            )
            .unwrap_err(),
            MouseSnapshotError::Closed
        );
    }

    #[test]
    fn duplicate_invalid_or_oversized_press_sets_are_rejected_without_allocating_a_key_cache() {
        let press = ObservedPress {
            button: Button::Key(65),
            ownership: ObservationOwnership::Passed,
        };
        for presses in [
            vec![press, press],
            vec![ObservedPress {
                button: Button::Key(0),
                ..press
            }],
            vec![press; 260],
        ] {
            let mut gate = MouseModifierGate::new(1, 0).unwrap();
            let e = event(1, 1, 0);
            assert_eq!(
                gate.observe(
                    e,
                    Some(snapshot(e)),
                    &presses,
                    MouseCaptureOwnership::default()
                )
                .unwrap_err(),
                MouseSnapshotError::InvalidPressSet
            );
        }
    }

    #[test]
    fn event_stream_ids_prevent_context_and_snapshot_reuse_between_gate_instances() {
        assert!(matches!(
            MouseModifierGate::new(0, 0),
            Err(MouseSnapshotError::InvalidStream)
        ));
        let mut first = MouseModifierGate::new(1, 0).unwrap();
        let first_event = event(1, 1, 100);
        let context = first
            .observe(
                first_event,
                Some(snapshot(first_event)),
                &[],
                MouseCaptureOwnership::default(),
            )
            .unwrap();
        let mut second = MouseModifierGate::new(2, 0).unwrap();
        assert_eq!(
            second
                .observe(
                    first_event,
                    Some(snapshot(first_event)),
                    &[],
                    MouseCaptureOwnership::default()
                )
                .unwrap_err(),
            MouseSnapshotError::EventStreamMismatch
        );
        let second_event = MouseContextEvent {
            stream_id: 2,
            ..first_event
        };
        assert!(
            second
                .observe(
                    second_event,
                    Some(snapshot(second_event)),
                    &[],
                    MouseCaptureOwnership::default()
                )
                .is_ok()
        );
        assert_eq!(
            second.matches(&context, policy()).unwrap_err(),
            MouseSnapshotError::ContextNoLongerCurrent
        );
        let third_event = MouseContextEvent {
            serial: 2,
            ..second_event
        };
        let wrong_sample = MouseModifierSnapshot {
            stream_id: 1,
            ..snapshot(third_event)
        };
        assert_eq!(
            second
                .observe(
                    third_event,
                    Some(wrong_sample),
                    &[],
                    MouseCaptureOwnership::default()
                )
                .unwrap_err(),
            MouseSnapshotError::SnapshotIdentityMismatch
        );
    }

    #[test]
    fn modifier_overlap_enumerates_side_states_without_assuming_other_modifiers_are_forbidden() {
        let control = policy();
        let alt = MouseModifierPolicy {
            required: ModifierRequirement::LeftAlt,
            ..policy()
        };
        assert!(!control.modifier_states_overlap(alt));
        assert!(
            MouseModifierPolicy {
                allow_other_modifiers: true,
                ..control
            }
            .modifier_states_overlap(MouseModifierPolicy {
                allow_other_modifiers: true,
                ..alt
            })
        );
        assert!(
            !MouseModifierPolicy {
                allow_other_modifiers: true,
                ..control
            }
            .modifier_states_overlap(alt)
        );
        assert!(control.modifier_states_overlap(MouseModifierPolicy {
            required: ModifierRequirement::LeftControl,
            ..control
        }));
        assert!(
            !MouseModifierPolicy {
                required: ModifierRequirement::LeftControl,
                ..control
            }
            .modifier_states_overlap(MouseModifierPolicy {
                required: ModifierRequirement::RightControl,
                ..control
            })
        );
    }

    #[test]
    fn snapshot_based_matching_has_real_overlap_with_empty_exact_unmodified_and_passed_modifier_rules()
     {
        let mut other = TriggerSpec {
            id: 1,
            held: vec![],
            activation: crate::Activation::Wheel(crate::WheelDirection::Up),
            pointer_scope: Default::default(),
            process_scope: Default::default(),
            wheel: Default::default(),
            keyboard: Default::default(),
            conditions: vec![],
        };
        assert!(policy().overlaps_existing_held(&other));
        other.held.push(Button::Key(65));
        assert!(!policy().overlaps_existing_held(&other));
        other.held.clear();
        other.keyboard.matching = KeyboardMatching::MouseUnmodified {
            priority: 0,
            pointer_scope_priority: true,
        };
        assert!(policy().overlaps_existing_held(&other));
        other.keyboard.matching = KeyboardMatching::MouseExact {
            priority: 0,
            pointer_scope_priority: true,
        };
        assert!(policy().overlaps_existing_held(&other));
        other.held.push(Button::Key(0x11));
        let left = MouseModifierPolicy {
            required: ModifierRequirement::LeftControl,
            ..policy()
        };
        assert!(left.overlaps_existing_held(&other));
        other.held[0] = Button::Key(0x5b);
        assert!(!policy().overlaps_existing_held(&other));
        assert!(
            MouseModifierPolicy {
                allow_other_modifiers: true,
                ..policy()
            }
            .overlaps_existing_held(&other)
        );
        other.keyboard.matching = KeyboardMatching::Captured { priority: 0 };
        assert!(!policy().overlaps_existing_held(&other));
        other.keyboard.matching = KeyboardMatching::MouseCapturedLeader {
            priority: 0,
            pointer_scope_priority: true,
        };
        assert!(!policy().overlaps_existing_held(&other));
    }

    #[test]
    fn packed_context_covers_all_valid_buttons_with_fixed_storage() {
        let mut presses: Vec<_> = (1..=254)
            .map(|key| ObservedPress {
                button: Button::Key(key),
                ownership: ObservationOwnership::Passed,
            })
            .collect();
        presses.extend(
            [
                MouseButton::Left,
                MouseButton::Right,
                MouseButton::Middle,
                MouseButton::X1,
                MouseButton::X2,
            ]
            .map(|mouse| ObservedPress {
                button: Button::Mouse(mouse),
                ownership: ObservationOwnership::Passed,
            }),
        );
        assert_eq!(presses.len(), MAX_PRESSES);
        let mut gate = MouseModifierGate::new(1, 0).unwrap();
        let e = event(1, 1, 100);
        let context = gate
            .observe(
                e,
                Some(snapshot(e)),
                &presses,
                MouseCaptureOwnership::default(),
            )
            .unwrap();
        assert!(
            gate.matches(
                &context,
                MouseModifierPolicy {
                    allow_other_modifiers: true,
                    allow_extra_passed_mouse: true,
                    allow_extra_passed_keys: true,
                    ..policy()
                }
            )
            .unwrap()
        );
        assert!(
            !gate
                .matches(
                    &context,
                    MouseModifierPolicy {
                        allow_other_modifiers: true,
                        allow_extra_passed_keys: true,
                        allow_extra_passed_mouse: false,
                        ..policy()
                    }
                )
                .unwrap()
        );
        assert_eq!(std::mem::size_of_val(&context.presses), 40);
    }

    #[test]
    fn snapshot_unmodified_domain_is_disjoint_from_control_alt_but_can_explicitly_allow_windows() {
        let unmodified = MouseModifierPolicy {
            required: ModifierRequirement::NoControlShiftAlt,
            allow_other_modifiers: true,
            allow_extra_passed_keys: true,
            allow_extra_passed_mouse: false,
        };
        assert!(!unmodified.modifier_states_overlap(policy()));
        assert!(!unmodified.modifier_states_overlap(MouseModifierPolicy {
            required: ModifierRequirement::LeftAlt,
            allow_other_modifiers: true,
            ..policy()
        }));
        assert!(unmodified.accepts_modifiers(ModifierSides::default()));
        assert!(unmodified.accepts_modifiers(ModifierSides {
            left_windows: true,
            ..Default::default()
        }));
        assert!(
            !MouseModifierPolicy {
                allow_other_modifiers: false,
                ..unmodified
            }
            .accepts_modifiers(ModifierSides {
                left_windows: true,
                ..Default::default()
            })
        );
        assert!(!unmodified.accepts_modifiers(ModifierSides {
            left_control: true,
            ..Default::default()
        }));
        assert!(!unmodified.accepts_passed_key(0x11));
        assert!(unmodified.accepts_passed_key(0x5b));
    }
}
