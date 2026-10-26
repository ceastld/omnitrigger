//! 与平台和界面无关的输入录制与触发器状态机。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    X1,
    X2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Button {
    Key(u16),
    Mouse(MouseButton),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Axis {
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WheelDirection {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputKind {
    Down(Button),
    Up(Button),
    Wheel { axis: Axis, delta: i32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InputEvent {
    pub kind: InputKind,
    pub injected: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Activation {
    Press(Button),
    Hold { button: Button, duration_ms: u32 },
    Wheel(WheelDirection),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TriggerSpec {
    pub id: u64,
    pub held: Vec<Button>,
    pub activation: Activation,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Decision {
    pub consume: bool,
    pub replay: Vec<InputKind>,
    pub triggered: Vec<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistrationError {
    TooManyBindings,
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
            Self::TooManyBindings => "最多可添加 64 个触发器，请删除不再使用的触发器后重试。",
            Self::TooManyHeld { .. } => "每个组合最多包含 8 个前缀按键，请减少同时按住的按键。",
            Self::InvalidButton { .. } => "组合包含不支持的按键，请重新录制。",
            Self::InvalidHoldDuration { .. } => "长按时间应为 50 至 10000 毫秒。",
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PressState {
    Pending,
    Consumed,
    Passed,
    Draining,
}

#[derive(Clone, Copy)]
struct PendingHold {
    id: u64,
    button: Button,
    deadline_ms: u64,
    pointer_origin: Option<(i32, i32)>,
}

/// 每个组合要求当前按下集合与 held 完全一致；多余按键不会被静默忽略。
pub struct Engine {
    bindings: Vec<TriggerSpec>,
    pressed: BTreeMap<Button, PressState>,
    pending: Vec<InputKind>,
    wheel: [i64; 4],
    enabled: bool,
    now_ms: u64,
    hold: Option<PendingHold>,
    pointer_position: Option<(i32, i32)>,
}

impl Engine {
    pub fn new(bindings: Vec<TriggerSpec>) -> Result<Self, RegistrationError> {
        Ok(Self {
            bindings: validate(bindings)?,
            pressed: BTreeMap::new(),
            pending: Vec::new(),
            wheel: [0; 4],
            enabled: true,
            now_ms: 0,
            hold: None,
            pointer_position: None,
        })
    }
    pub fn replace_bindings(
        &mut self,
        bindings: Vec<TriggerSpec>,
    ) -> Result<(), RegistrationError> {
        let bindings = validate(bindings)?;
        self.cancel();
        self.bindings = bindings;
        Ok(())
    }
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.cancel();
            self.enabled = enabled;
        }
    }
    pub fn cancel(&mut self) {
        self.pending.clear();
        self.hold = None;
        self.wheel = [0; 4];
        for state in self.pressed.values_mut() {
            if matches!(*state, PressState::Pending | PressState::Consumed) {
                *state = PressState::Draining;
            }
        }
    }
    /// 使用最近一次显式提供的时间；本方法不会读取系统时钟。
    pub fn process(&mut self, event: InputEvent) -> Decision {
        self.process_at(event, self.now_ms)
    }

    /// 先推进计时，再处理当前物理事件。注入事件完全忽略，不推进时间。
    pub fn process_at(&mut self, event: InputEvent, now_ms: u64) -> Decision {
        if event.injected {
            return Decision::default();
        }
        let mut decision = self.advance_time(now_ms);
        let interrupted = self.hold.is_some_and(|hold| match event.kind {
            InputKind::Down(button) => button != hold.button && !self.pressed.contains_key(&button),
            // 释放由 up 按原 Pending/Consumed 语义回放并收尾，避免先转 Passed。
            InputKind::Up(_) => false,
            InputKind::Wheel { delta, .. } => delta != 0,
        });
        if interrupted {
            decision.replay.extend(self.flush_pending());
        }
        let current = match event.kind {
            InputKind::Down(button) => self.down(button),
            InputKind::Up(button) => self.up(button),
            InputKind::Wheel { axis, delta } => self.scroll(axis, delta),
        };
        decision.consume = current.consume;
        decision.replay.extend(current.replay);
        decision.triggered.extend(current.triggered);
        decision
    }

    /// 时间单调前进；倒退的时间戳按最近时间处理，避免重复触发或延长等待。
    pub fn advance_time(&mut self, now_ms: u64) -> Decision {
        self.now_ms = self.now_ms.max(now_ms);
        let Some(hold) = self.hold else {
            return Decision::default();
        };
        if self.now_ms < hold.deadline_ms {
            return Decision::default();
        }
        self.commit_pending();
        Decision {
            triggered: vec![hold.id],
            ..Decision::default()
        }
    }

    pub fn next_deadline_ms(&self) -> Option<u64> {
        self.hold.map(|hold| hold.deadline_ms)
    }

    /// 只传入物理鼠标位置；平台应在鼠标按下前同步位置，供长按记录起点。
    /// 鼠标长按在 8 像素半径外取消并回放缓冲；键盘长按不受鼠标移动影响。
    pub fn pointer_moved(&mut self, position: (i32, i32), now_ms: u64) -> Decision {
        let mut decision = self.advance_time(now_ms);
        self.pointer_position = Some(position);
        let moved = match self.hold.as_mut() {
            Some(hold) if matches!(hold.button, Button::Mouse(_)) => {
                if let Some(origin) = hold.pointer_origin {
                    let dx = i64::from(position.0) - i64::from(origin.0);
                    let dy = i64::from(position.1) - i64::from(origin.1);
                    // 先限制各轴，平方运算只处理 -8..=8，极端坐标也不会溢出。
                    dx.abs() > 8 || dy.abs() > 8 || dx * dx + dy * dy > 64
                } else {
                    hold.pointer_origin = Some(position);
                    false
                }
            }
            _ => false,
        };
        if moved {
            decision.replay.extend(self.flush_pending());
        }
        decision
    }

    fn down(&mut self, button: Button) -> Decision {
        if let Some(state) = self.pressed.get(&button) {
            // 自动重复不是第二次按压；已消费的按压也不能通过重复事件泄漏。
            return Decision {
                consume: *state != PressState::Passed,
                ..Decision::default()
            };
        }
        if !self.enabled {
            self.pressed.insert(button, PressState::Passed);
            return Decision::default();
        }
        if let Some(id) = self
            .bindings
            .iter()
            .find(|binding| {
                binding.activation == Activation::Press(button) && self.held_matches(&binding.held)
            })
            .map(|binding| binding.id)
        {
            self.commit_pending();
            self.pressed.insert(button, PressState::Consumed);
            return Decision {
                consume: true,
                triggered: vec![id],
                ..Decision::default()
            };
        }
        if let Some((id, duration_ms)) =
            self.bindings
                .iter()
                .find_map(|binding| match binding.activation {
                    Activation::Hold {
                        button: terminal,
                        duration_ms,
                    } if terminal == button && self.held_matches(&binding.held) => {
                        Some((binding.id, duration_ms))
                    }
                    _ => None,
                })
        {
            self.wheel = [0; 4];
            self.pressed.insert(button, PressState::Pending);
            self.pending.push(InputKind::Down(button));
            self.hold = Some(PendingHold {
                id,
                button,
                deadline_ms: self.now_ms.saturating_add(u64::from(duration_ms)),
                pointer_origin: self.pointer_position,
            });
            return Decision {
                consume: true,
                ..Decision::default()
            };
        }
        let eligible = self
            .pressed
            .values()
            .all(|state| matches!(state, PressState::Pending | PressState::Consumed));
        let extends_prefix = eligible
            && self.bindings.iter().any(|binding| {
                binding.held.contains(&button)
                    && self
                        .pressed
                        .keys()
                        .all(|pressed| binding.held.contains(pressed))
            });
        if extends_prefix {
            self.wheel = [0; 4];
            self.pressed.insert(button, PressState::Pending);
            self.pending.push(InputKind::Down(button));
            return Decision {
                consume: true,
                ..Decision::default()
            };
        }
        let replay = self.flush_pending();
        self.pressed.insert(button, PressState::Passed);
        Decision {
            replay,
            ..Decision::default()
        }
    }

    fn up(&mut self, button: Button) -> Decision {
        let Some(state) = self.pressed.remove(&button) else {
            return Decision::default();
        };
        self.wheel = [0; 4];
        match state {
            PressState::Pending => {
                let mut replay = self.flush_pending();
                replay.push(InputKind::Up(button));
                Decision {
                    consume: true,
                    replay,
                    ..Decision::default()
                }
            }
            PressState::Consumed => {
                // 已完成组合的前缀释放后，不让另一个尚未完成的组合继续等待。
                let replay = self.flush_pending();
                Decision {
                    consume: true,
                    replay,
                    ..Decision::default()
                }
            }
            PressState::Draining => Decision {
                consume: true,
                ..Decision::default()
            },
            PressState::Passed => Decision::default(),
        }
    }

    fn scroll(&mut self, axis: Axis, delta: i32) -> Decision {
        if delta == 0 || !self.enabled {
            return Decision::default();
        }
        let direction = direction(axis, delta);
        let Some(id) = self
            .bindings
            .iter()
            .find(|binding| {
                binding.activation == Activation::Wheel(direction)
                    && self.held_matches(&binding.held)
            })
            .map(|binding| binding.id)
        else {
            return Decision {
                replay: self.flush_pending(),
                ..Decision::default()
            };
        };
        let index = wheel_index(direction);
        self.wheel[index] = self.wheel[index].saturating_add(i64::from(delta).abs());
        // 一条异常大的公共API输入不能让hook同步路径产生无界工作；剩余步数保留。
        let count = (self.wheel[index] / 120).min(64) as usize;
        self.wheel[index] -= count as i64 * 120;
        if count > 0 {
            self.commit_pending();
        } else if self
            .pressed
            .values()
            .any(|state| *state == PressState::Pending)
        {
            // 未命中时回放真实增量，避免丢掉高精度滚轮的半格输入。
            self.pending.push(InputKind::Wheel { axis, delta });
        }
        Decision {
            consume: true,
            triggered: vec![id; count],
            ..Decision::default()
        }
    }

    fn held_matches(&self, held: &[Button]) -> bool {
        self.pressed.len() == held.len()
            && self.pressed.iter().all(|(button, state)| {
                matches!(state, PressState::Pending | PressState::Consumed) && held.contains(button)
            })
    }

    fn commit_pending(&mut self) {
        self.pending.clear();
        self.hold = None;
        for state in self.pressed.values_mut() {
            if *state == PressState::Pending {
                *state = PressState::Consumed;
            }
        }
    }

    fn flush_pending(&mut self) -> Vec<InputKind> {
        self.hold = None;
        self.wheel = [0; 4];
        for state in self.pressed.values_mut() {
            if *state == PressState::Pending {
                *state = PressState::Passed;
            }
        }
        std::mem::take(&mut self.pending)
    }
}

fn validate(mut bindings: Vec<TriggerSpec>) -> Result<Vec<TriggerSpec>, RegistrationError> {
    if bindings.len() > 64 {
        return Err(RegistrationError::TooManyBindings);
    }
    let mut ids = BTreeSet::new();
    for binding in &mut bindings {
        if !ids.insert(binding.id) {
            return Err(RegistrationError::DuplicateId { id: binding.id });
        }
        if binding.held.len() > 8 {
            return Err(RegistrationError::TooManyHeld { id: binding.id });
        }
        for &button in binding.held.iter().chain(match &binding.activation {
            Activation::Press(button) | Activation::Hold { button, .. } => {
                std::slice::from_ref(button)
            }
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
            let same_button = activation_button(left.activation)
                .is_some_and(|button| Some(button) == activation_button(right.activation));
            if left.held == right.held && (left.activation == right.activation || same_button) {
                return Err(RegistrationError::DuplicateTrigger {
                    first: left.id,
                    second: right.id,
                });
            }
        }
        if let Some(button) = activation_button(left.activation)
            && let Some(prefix) = bindings.iter().find(|other| other.held.contains(&button))
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
        Activation::Press(button) | Activation::Hold { button, .. } => Some(button),
        Activation::Wheel(_) => None,
    }
}

fn direction(axis: Axis, delta: i32) -> WheelDirection {
    match (axis, delta > 0) {
        (Axis::Vertical, true) => WheelDirection::Up,
        (Axis::Vertical, false) => WheelDirection::Down,
        (Axis::Horizontal, true) => WheelDirection::Right,
        (Axis::Horizontal, false) => WheelDirection::Left,
    }
}

fn wheel_index(direction: WheelDirection) -> usize {
    match direction {
        WheelDirection::Up => 0,
        WheelDirection::Down => 1,
        WheelDirection::Left => 2,
        WheelDirection::Right => 3,
    }
}

#[derive(Default)]
pub struct Recorder {
    pressed: BTreeSet<Button>,
    candidate: Option<TriggerSpec>,
    completed: bool,
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn process(&mut self, event: InputEvent) -> Option<TriggerSpec> {
        if event.injected || self.completed {
            return None;
        }
        match event.kind {
            InputKind::Down(button) => {
                if self.pressed.insert(button) {
                    self.candidate = Some(TriggerSpec {
                        id: 0,
                        held: self
                            .pressed
                            .iter()
                            .copied()
                            .filter(|held| *held != button)
                            .collect(),
                        activation: Activation::Press(button),
                    });
                }
                None
            }
            InputKind::Up(button) => {
                if self.pressed.remove(&button) && self.pressed.is_empty() {
                    self.completed = true;
                    self.candidate.take()
                } else {
                    None
                }
            }
            InputKind::Wheel { axis, delta } if delta != 0 => {
                self.completed = true;
                Some(TriggerSpec {
                    id: 0,
                    held: self.pressed.iter().copied().collect(),
                    activation: Activation::Wheel(direction(axis, delta)),
                })
            }
            InputKind::Wheel { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAPS: Button = Button::Key(0x14);
    const SHIFT: Button = Button::Key(0xA0);
    const W: Button = Button::Key(0x57);
    const Q: Button = Button::Key(0x51);
    const SIDE: Button = Button::Mouse(MouseButton::X1);
    const RIGHT: Button = Button::Mouse(MouseButton::Right);

    fn event(kind: InputKind) -> InputEvent {
        InputEvent {
            kind,
            injected: false,
        }
    }
    fn press(id: u64, held: &[Button], terminal: Button) -> TriggerSpec {
        TriggerSpec {
            id,
            held: held.to_vec(),
            activation: Activation::Press(terminal),
        }
    }
    fn wheel(id: u64, held: &[Button], direction: WheelDirection) -> TriggerSpec {
        TriggerSpec {
            id,
            held: held.to_vec(),
            activation: Activation::Wheel(direction),
        }
    }
    fn hold(id: u64, held: &[Button], button: Button, duration_ms: u32) -> TriggerSpec {
        TriggerSpec {
            id,
            held: held.to_vec(),
            activation: Activation::Hold {
                button,
                duration_ms,
            },
        }
    }
    fn scroll(delta: i32) -> InputEvent {
        event(InputKind::Wheel {
            axis: Axis::Vertical,
            delta,
        })
    }

    #[test]
    fn caps_chord_consumes_every_press_edge_and_ignores_repeat() {
        let mut engine = Engine::new(vec![press(1, &[CAPS], W)]).unwrap();
        assert!(engine.process(event(InputKind::Down(CAPS))).consume);
        assert!(engine.process(event(InputKind::Down(CAPS))).consume);
        assert_eq!(engine.process(event(InputKind::Down(W))).triggered, vec![1]);
        let repeat = engine.process(event(InputKind::Down(W)));
        assert!(repeat.consume && repeat.triggered.is_empty());
        assert!(engine.process(event(InputKind::Up(CAPS))).consume);
        assert!(engine.process(event(InputKind::Up(W))).consume);
        assert!(!engine.process(event(InputKind::Up(W))).consume);
    }

    #[test]
    fn standalone_prefix_replays_exactly_one_down_and_up() {
        let mut engine = Engine::new(vec![press(1, &[CAPS], W)]).unwrap();
        engine.process(event(InputKind::Down(CAPS)));
        engine.process(event(InputKind::Down(CAPS)));
        assert_eq!(
            engine.process(event(InputKind::Up(CAPS))),
            Decision {
                consume: true,
                replay: vec![InputKind::Down(CAPS), InputKind::Up(CAPS)],
                triggered: vec![],
            }
        );
    }

    #[test]
    fn unmatched_successor_flushes_before_passthrough_and_disarms_prefix() {
        let mut engine = Engine::new(vec![press(1, &[CAPS], W)]).unwrap();
        engine.process(event(InputKind::Down(CAPS)));
        let fallback = engine.process(event(InputKind::Down(Q)));
        assert!(!fallback.consume);
        assert_eq!(fallback.replay, vec![InputKind::Down(CAPS)]);
        assert!(!engine.process(event(InputKind::Up(Q))).consume);
        assert_eq!(
            engine.process(event(InputKind::Down(W))),
            Decision::default()
        );
        assert!(!engine.process(event(InputKind::Up(W))).consume);
        assert!(!engine.process(event(InputKind::Up(CAPS))).consume);
    }

    #[test]
    fn multiple_prefixes_accept_either_order_and_release_fallback_stays_paired() {
        let mut engine = Engine::new(vec![press(1, &[CAPS, SHIFT], W)]).unwrap();
        engine.process(event(InputKind::Down(SHIFT)));
        engine.process(event(InputKind::Down(CAPS)));
        let fallback = engine.process(event(InputKind::Up(SHIFT)));
        assert!(fallback.consume);
        assert_eq!(
            fallback.replay,
            vec![
                InputKind::Down(SHIFT),
                InputKind::Down(CAPS),
                InputKind::Up(SHIFT)
            ]
        );
        assert!(!engine.process(event(InputKind::Up(CAPS))).consume);
        engine.process(event(InputKind::Down(CAPS)));
        engine.process(event(InputKind::Down(SHIFT)));
        assert_eq!(engine.process(event(InputKind::Down(W))).triggered, vec![1]);
    }

    #[test]
    fn matching_uses_exact_pressed_set() {
        let mut engine =
            Engine::new(vec![press(1, &[CAPS], W), press(2, &[CAPS, SHIFT], W)]).unwrap();
        engine.process(event(InputKind::Down(CAPS)));
        engine.process(event(InputKind::Down(SHIFT)));
        assert_eq!(engine.process(event(InputKind::Down(W))).triggered, vec![2]);
    }

    #[test]
    fn side_button_wheel_accumulates_and_consumes_prefix_release() {
        let mut engine = Engine::new(vec![wheel(9, &[SIDE], WheelDirection::Up)]).unwrap();
        engine.process(event(InputKind::Down(SIDE)));
        let partial = engine.process(scroll(60));
        assert!(partial.consume && partial.triggered.is_empty());
        assert_eq!(engine.process(scroll(60)).triggered, vec![9]);
        assert_eq!(engine.process(scroll(240)).triggered, vec![9, 9]);
        let release = engine.process(event(InputKind::Up(SIDE)));
        assert!(release.consume && release.replay.is_empty());
    }

    #[test]
    fn partial_wheel_is_replayed_on_unused_prefix_release() {
        let mut engine = Engine::new(vec![wheel(9, &[SIDE], WheelDirection::Up)]).unwrap();
        engine.process(event(InputKind::Down(SIDE)));
        engine.process(scroll(60));
        assert_eq!(
            engine.process(event(InputKind::Up(SIDE))).replay,
            vec![
                InputKind::Down(SIDE),
                InputKind::Wheel {
                    axis: Axis::Vertical,
                    delta: 60
                },
                InputKind::Up(SIDE),
            ]
        );
    }

    #[test]
    fn opposite_directions_have_independent_accumulators() {
        let mut engine = Engine::new(vec![
            wheel(9, &[SIDE], WheelDirection::Up),
            wheel(10, &[SIDE], WheelDirection::Down),
        ])
        .unwrap();
        engine.process(event(InputKind::Down(SIDE)));
        assert!(engine.process(scroll(60)).triggered.is_empty());
        assert!(engine.process(scroll(-60)).triggered.is_empty());
        assert_eq!(engine.process(scroll(60)).triggered, vec![9]);
        assert_eq!(engine.process(scroll(-60)).triggered, vec![10]);
    }

    #[test]
    fn unmatched_wheel_replays_partial_before_current_input() {
        let mut engine = Engine::new(vec![wheel(9, &[SIDE], WheelDirection::Up)]).unwrap();
        engine.process(event(InputKind::Down(SIDE)));
        engine.process(scroll(60));
        let decision = engine.process(scroll(-120));
        assert!(!decision.consume);
        assert_eq!(
            decision.replay,
            vec![
                InputKind::Down(SIDE),
                InputKind::Wheel {
                    axis: Axis::Vertical,
                    delta: 60
                }
            ]
        );
        assert!(!engine.process(event(InputKind::Up(SIDE))).consume);
    }

    #[test]
    fn adding_another_prefix_does_not_borrow_wheel_delta_from_previous_chord() {
        let mut engine = Engine::new(vec![
            wheel(9, &[SIDE], WheelDirection::Up),
            wheel(10, &[SIDE, SHIFT], WheelDirection::Up),
        ])
        .unwrap();
        engine.process(event(InputKind::Down(SIDE)));
        engine.process(scroll(60));
        engine.process(event(InputKind::Down(SHIFT)));
        assert!(engine.process(scroll(60)).triggered.is_empty());
        assert_eq!(engine.process(scroll(60)).triggered, vec![10]);
    }

    #[test]
    fn horizontal_wheel_and_extreme_delta_are_bounded() {
        let mut engine = Engine::new(vec![wheel(1, &[], WheelDirection::Left)]).unwrap();
        assert_eq!(
            engine
                .process(event(InputKind::Wheel {
                    axis: Axis::Horizontal,
                    delta: -120
                }))
                .triggered,
            vec![1]
        );
        assert_eq!(
            engine
                .process(event(InputKind::Wheel {
                    axis: Axis::Horizontal,
                    delta: i32::MIN
                }))
                .triggered
                .len(),
            64
        );
        engine.cancel();
        assert!(
            engine
                .process(event(InputKind::Wheel {
                    axis: Axis::Horizontal,
                    delta: -1
                }))
                .triggered
                .is_empty()
        );
    }

    #[test]
    fn injected_events_are_passthrough_and_cannot_mutate_physical_state() {
        let mut engine = Engine::new(vec![press(1, &[CAPS], W)]).unwrap();
        engine.process(event(InputKind::Down(CAPS)));
        let injected = InputEvent {
            kind: InputKind::Up(CAPS),
            injected: true,
        };
        assert_eq!(engine.process(injected), Decision::default());
        assert_eq!(engine.process(event(InputKind::Down(W))).triggered, vec![1]);
    }

    #[test]
    fn disable_drains_suppressed_up_but_preserves_passed_up() {
        let mut engine = Engine::new(vec![press(1, &[CAPS], W)]).unwrap();
        engine.process(event(InputKind::Down(CAPS)));
        engine.set_enabled(false);
        assert!(engine.process(event(InputKind::Down(CAPS))).consume);
        assert!(engine.process(event(InputKind::Up(CAPS))).consume);
        assert!(!engine.process(event(InputKind::Down(W))).consume);
        engine.set_enabled(true);
        assert!(!engine.process(event(InputKind::Up(W))).consume);
    }

    #[test]
    fn cancel_and_replace_cannot_reuse_a_draining_press() {
        let mut engine = Engine::new(vec![press(1, &[CAPS], W)]).unwrap();
        engine.process(event(InputKind::Down(CAPS)));
        engine.replace_bindings(vec![press(2, &[CAPS], Q)]).unwrap();
        assert!(
            engine
                .process(event(InputKind::Down(Q)))
                .triggered
                .is_empty()
        );
        assert!(engine.process(event(InputKind::Up(CAPS))).consume);
        engine.process(event(InputKind::Up(Q)));
        engine.process(event(InputKind::Down(CAPS)));
        assert_eq!(engine.process(event(InputKind::Down(Q))).triggered, vec![2]);
    }

    #[test]
    fn invalid_replacement_is_transactional() {
        let mut engine = Engine::new(vec![press(1, &[CAPS], W)]).unwrap();
        engine.process(event(InputKind::Down(CAPS)));
        assert!(
            engine
                .replace_bindings(vec![press(2, &[CAPS, CAPS], Q)])
                .is_err()
        );
        assert_eq!(engine.process(event(InputKind::Down(W))).triggered, vec![1]);
    }

    #[test]
    fn registration_rejects_ambiguous_invalid_and_excessive_inputs() {
        assert!(matches!(
            Engine::new(vec![press(1, &[], CAPS), press(2, &[CAPS], W)]),
            Err(RegistrationError::TerminalIsPrefix { .. })
        ));
        assert!(matches!(
            Engine::new(vec![press(1, &[CAPS], W), press(2, &[CAPS], W)]),
            Err(RegistrationError::DuplicateTrigger { .. })
        ));
        assert!(matches!(
            Engine::new(vec![press(1, &[], W), press(1, &[], Q)]),
            Err(RegistrationError::DuplicateId { .. })
        ));
        assert!(matches!(
            Engine::new(vec![press(1, &[CAPS], CAPS)]),
            Err(RegistrationError::TerminalInHeld { .. })
        ));
        assert!(matches!(
            Engine::new(vec![press(1, &[], Button::Key(0))]),
            Err(RegistrationError::InvalidButton { .. })
        ));
        assert!(matches!(
            Engine::new(vec![press(1, &[], W); 65]),
            Err(RegistrationError::TooManyBindings)
        ));
        let many_held = (1..=9).map(Button::Key).collect::<Vec<_>>();
        assert!(matches!(
            Engine::new(vec![press(1, &many_held, W)]),
            Err(RegistrationError::TooManyHeld { .. })
        ));
    }

    #[test]
    fn recording_chord_waits_for_all_releases_and_ignores_mouse_up_from_start_button() {
        let mut recorder = Recorder::new();
        assert!(
            recorder
                .process(event(InputKind::Up(Button::Mouse(MouseButton::Left))))
                .is_none()
        );
        assert!(recorder.process(event(InputKind::Down(CAPS))).is_none());
        assert!(recorder.process(event(InputKind::Down(W))).is_none());
        assert!(recorder.process(event(InputKind::Down(W))).is_none());
        assert!(recorder.process(event(InputKind::Up(CAPS))).is_none());
        assert_eq!(
            recorder.process(event(InputKind::Up(W))),
            Some(press(0, &[CAPS], W))
        );
        assert!(recorder.process(event(InputKind::Down(Q))).is_none());
        assert!(recorder.process(event(InputKind::Up(Q))).is_none());
    }

    #[test]
    fn recorder_wheel_uses_first_nonzero_direction_and_reset_starts_new_recording() {
        let mut recorder = Recorder::new();
        assert!(
            recorder
                .process(InputEvent {
                    kind: InputKind::Down(CAPS),
                    injected: true
                })
                .is_none()
        );
        assert!(recorder.process(scroll(0)).is_none());
        recorder.process(event(InputKind::Down(SIDE)));
        assert_eq!(
            recorder.process(scroll(-1)),
            Some(wheel(0, &[SIDE], WheelDirection::Down))
        );
        assert!(recorder.process(scroll(120)).is_none());
        recorder.reset();
        recorder.process(event(InputKind::Down(Q)));
        assert_eq!(
            recorder.process(event(InputKind::Up(Q))),
            Some(press(0, &[], Q))
        );
    }

    #[test]
    fn hold_timer_fires_once_without_more_input_and_consumes_its_release() {
        let mut engine = Engine::new(vec![hold(1, &[], RIGHT, 300)]).unwrap();
        assert!(
            engine
                .process_at(event(InputKind::Down(RIGHT)), 1000)
                .consume
        );
        assert_eq!(engine.next_deadline_ms(), Some(1300));
        assert_eq!(engine.advance_time(1299), Decision::default());
        assert_eq!(
            engine.advance_time(1300),
            Decision {
                triggered: vec![1],
                ..Decision::default()
            }
        );
        assert_eq!(engine.next_deadline_ms(), None);
        assert_eq!(engine.advance_time(5000), Decision::default());
        let repeat = engine.process_at(event(InputKind::Down(RIGHT)), 5000);
        assert!(repeat.consume && repeat.triggered.is_empty());
        let release = engine.process_at(event(InputKind::Up(RIGHT)), 5001);
        assert!(release.consume && release.replay.is_empty());
        assert!(!engine.process_at(event(InputKind::Up(RIGHT)), 5002).consume);
    }

    #[test]
    fn hold_release_before_deadline_replays_both_edges_but_exact_deadline_fires() {
        for (release_at, fired) in [(299, false), (300, true), (1000, true)] {
            let mut engine = Engine::new(vec![hold(1, &[], RIGHT, 300)]).unwrap();
            engine.process_at(event(InputKind::Down(RIGHT)), 0);
            let release = engine.process_at(event(InputKind::Up(RIGHT)), release_at);
            assert!(release.consume);
            if fired {
                assert_eq!(release.triggered, vec![1]);
                assert!(release.replay.is_empty());
            } else {
                assert!(release.triggered.is_empty());
                assert_eq!(
                    release.replay,
                    vec![InputKind::Down(RIGHT), InputKind::Up(RIGHT)]
                );
            }
            assert_eq!(engine.next_deadline_ms(), None);
            assert_eq!(engine.advance_time(2000), Decision::default());
        }
    }

    #[test]
    fn keyboard_hold_repeat_and_backwards_time_do_not_restart_the_deadline() {
        let mut engine = Engine::new(vec![hold(1, &[], W, 300)]).unwrap();
        engine.advance_time(1000);
        assert!(engine.process(event(InputKind::Down(W))).consume);
        assert_eq!(engine.next_deadline_ms(), Some(1300));
        assert!(engine.process_at(event(InputKind::Down(W)), 1200).consume);
        assert!(engine.process_at(event(InputKind::Down(W)), 1100).consume);
        assert_eq!(engine.next_deadline_ms(), Some(1300));
        assert_eq!(engine.advance_time(1299), Decision::default());
        assert_eq!(engine.advance_time(1300).triggered, vec![1]);
        assert!(engine.process(event(InputKind::Up(W))).consume);
    }

    #[test]
    fn injected_input_cannot_advance_cancel_or_release_a_hold() {
        let mut engine = Engine::new(vec![hold(1, &[], W, 300)]).unwrap();
        engine.process_at(event(InputKind::Down(W)), 1000);
        for kind in [InputKind::Up(W), InputKind::Down(Q)] {
            assert_eq!(
                engine.process_at(
                    InputEvent {
                        kind,
                        injected: true,
                    },
                    5000,
                ),
                Decision::default()
            );
        }
        assert_eq!(engine.next_deadline_ms(), Some(1300));
        assert_eq!(engine.advance_time(1299), Decision::default());
        assert_eq!(engine.advance_time(1300).triggered, vec![1]);
        assert!(engine.process_at(event(InputKind::Up(W)), 1301).consume);
    }

    #[test]
    fn mouse_hold_has_an_eight_pixel_radius_including_negative_coordinates() {
        let mut engine = Engine::new(vec![hold(1, &[], RIGHT, 300)]).unwrap();
        engine.pointer_moved((-1920, -100), 0);
        engine.process_at(event(InputKind::Down(RIGHT)), 0);
        assert_eq!(
            engine.pointer_moved((-1912, -100), 100),
            Decision::default()
        );
        assert_eq!(
            engine.pointer_moved((-1920, -108), 200),
            Decision::default()
        );
        let moved = engine.pointer_moved((-1914, -94), 299);
        assert!(!moved.consume);
        assert_eq!(moved.replay, vec![InputKind::Down(RIGHT)]);
        assert_eq!(engine.next_deadline_ms(), None);
        assert_eq!(engine.advance_time(300), Decision::default());
        assert!(!engine.process_at(event(InputKind::Up(RIGHT)), 301).consume);
    }

    #[test]
    fn extreme_pointer_coordinates_do_not_overflow_and_only_mouse_holds_cancel() {
        for button in [RIGHT, W] {
            let mut engine = Engine::new(vec![hold(1, &[], button, 300)]).unwrap();
            engine.pointer_moved((i32::MIN, i32::MIN), 0);
            engine.process_at(event(InputKind::Down(button)), 0);
            let moved = engine.pointer_moved((i32::MAX, i32::MAX), 299);
            if button == RIGHT {
                assert_eq!(moved.replay, vec![InputKind::Down(RIGHT)]);
                assert_eq!(engine.advance_time(300), Decision::default());
                assert!(!engine.process_at(event(InputKind::Up(RIGHT)), 301).consume);
            } else {
                assert_eq!(moved, Decision::default());
                assert_eq!(engine.advance_time(300).triggered, vec![1]);
                assert!(engine.process_at(event(InputKind::Up(W)), 301).consume);
            }
        }
    }

    #[test]
    fn pointer_motion_at_the_deadline_fires_before_processing_the_motion() {
        let mut engine = Engine::new(vec![hold(1, &[], RIGHT, 300)]).unwrap();
        engine.pointer_moved((0, 0), 0);
        engine.process_at(event(InputKind::Down(RIGHT)), 0);
        let moved = engine.pointer_moved((1000, 1000), 300);
        assert_eq!(moved.triggered, vec![1]);
        assert!(moved.replay.is_empty());
        assert!(engine.process_at(event(InputKind::Up(RIGHT)), 301).consume);
    }

    #[test]
    fn another_button_or_nonzero_wheel_cancels_and_hands_the_hold_back() {
        let mut engine = Engine::new(vec![hold(1, &[], RIGHT, 300)]).unwrap();
        engine.process_at(event(InputKind::Down(RIGHT)), 0);
        let interrupted = engine.process_at(event(InputKind::Down(Q)), 100);
        assert!(!interrupted.consume);
        assert_eq!(interrupted.replay, vec![InputKind::Down(RIGHT)]);
        assert!(!engine.process_at(event(InputKind::Up(Q)), 101).consume);
        assert!(!engine.process_at(event(InputKind::Up(RIGHT)), 102).consume);
        assert_eq!(engine.advance_time(300), Decision::default());
        engine.process_at(event(InputKind::Down(RIGHT)), 400);
        assert_eq!(engine.process_at(scroll(0), 499), Decision::default());
        assert_eq!(engine.next_deadline_ms(), Some(700));
        let wheel = engine.process_at(scroll(120), 500);
        assert!(!wheel.consume);
        assert_eq!(wheel.replay, vec![InputKind::Down(RIGHT)]);
        assert!(!engine.process_at(event(InputKind::Up(RIGHT)), 501).consume);
        assert_eq!(engine.advance_time(700), Decision::default());
        engine.process_at(event(InputKind::Down(RIGHT)), 800);
        assert_eq!(engine.advance_time(1100).triggered, vec![1]);
    }

    #[test]
    fn hold_prefixes_use_the_exact_set_and_release_fallback_stays_paired() {
        let mut engine = Engine::new(vec![hold(1, &[CAPS], W, 300)]).unwrap();
        engine.process_at(event(InputKind::Down(SHIFT)), 0);
        engine.process_at(event(InputKind::Down(CAPS)), 1);
        engine.process_at(event(InputKind::Down(W)), 2);
        assert_eq!(engine.next_deadline_ms(), None);
        for button in [SHIFT, CAPS, W] {
            engine.process_at(event(InputKind::Up(button)), 3);
        }
        engine.process_at(event(InputKind::Down(CAPS)), 100);
        engine.process_at(event(InputKind::Down(W)), 200);
        assert_eq!(engine.next_deadline_ms(), Some(500));
        // 已按住前缀的重复事件同样不是新的按压。
        engine.process_at(event(InputKind::Down(CAPS)), 250);
        assert_eq!(engine.next_deadline_ms(), Some(500));
        let release = engine.process_at(event(InputKind::Up(CAPS)), 300);
        assert!(release.consume);
        assert_eq!(
            release.replay,
            vec![
                InputKind::Down(CAPS),
                InputKind::Down(W),
                InputKind::Up(CAPS)
            ]
        );
        assert_eq!(engine.next_deadline_ms(), None);
        assert!(!engine.process_at(event(InputKind::Up(W)), 301).consume);
        engine.process_at(event(InputKind::Down(CAPS)), 600);
        engine.process_at(event(InputKind::Down(W)), 700);
        assert_eq!(engine.advance_time(1000).triggered, vec![1]);
        assert!(engine.process_at(event(InputKind::Up(CAPS)), 1001).consume);
        assert!(engine.process_at(event(InputKind::Up(W)), 1002).consume);
    }

    #[test]
    fn cancel_disable_and_replace_drop_hold_buffers_and_drain_releases() {
        let mut engine = Engine::new(vec![hold(1, &[], RIGHT, 300)]).unwrap();
        for mode in 0..3 {
            let start = mode * 1000;
            engine.process_at(event(InputKind::Down(RIGHT)), start);
            match mode {
                0 => engine.cancel(),
                1 => engine.set_enabled(false),
                _ => engine
                    .replace_bindings(vec![hold(2, &[], RIGHT, 300)])
                    .unwrap(),
            }
            assert_eq!(engine.next_deadline_ms(), None);
            assert_eq!(engine.advance_time(start + 500), Decision::default());
            let release = engine.process_at(event(InputKind::Up(RIGHT)), start + 501);
            assert!(release.consume && release.replay.is_empty());
            engine.set_enabled(true);
        }
        engine.process_at(event(InputKind::Down(RIGHT)), 3000);
        assert_eq!(engine.advance_time(3300).triggered, vec![2]);
    }

    #[test]
    fn invalid_hold_replacement_preserves_the_original_pending_deadline() {
        let mut engine = Engine::new(vec![hold(1, &[], RIGHT, 300)]).unwrap();
        engine.process_at(event(InputKind::Down(RIGHT)), 1000);
        assert_eq!(
            engine.replace_bindings(vec![hold(2, &[], RIGHT, 49)]),
            Err(RegistrationError::InvalidHoldDuration {
                id: 2,
                duration_ms: 49,
            })
        );
        assert_eq!(engine.next_deadline_ms(), Some(1300));
        assert_eq!(engine.advance_time(1300).triggered, vec![1]);
    }

    #[test]
    fn hold_registration_rejects_bad_durations_and_ambiguous_bindings() {
        for duration_ms in [0, 49, 10001, u32::MAX] {
            assert!(matches!(
                Engine::new(vec![hold(1, &[], RIGHT, duration_ms)]),
                Err(RegistrationError::InvalidHoldDuration {
                    id: 1,
                    duration_ms: value,
                }) if value == duration_ms
            ));
        }
        for duration_ms in [50, 10000] {
            assert!(Engine::new(vec![hold(1, &[], RIGHT, duration_ms)]).is_ok());
        }
        for other in [press(2, &[], RIGHT), hold(2, &[], RIGHT, 500)] {
            assert!(matches!(
                Engine::new(vec![hold(1, &[], RIGHT, 300), other]),
                Err(RegistrationError::DuplicateTrigger {
                    first: 1,
                    second: 2,
                })
            ));
        }
        assert!(matches!(
            Engine::new(vec![
                hold(1, &[], RIGHT, 300),
                wheel(2, &[RIGHT], WheelDirection::Up)
            ]),
            Err(RegistrationError::TerminalIsPrefix {
                terminal_binding: 1,
                prefix_binding: 2,
            })
        ));
        assert!(matches!(
            Engine::new(vec![hold(1, &[W], W, 300)]),
            Err(RegistrationError::TerminalInHeld { id: 1 })
        ));
        assert!(matches!(
            Engine::new(vec![hold(1, &[], Button::Key(0), 300)]),
            Err(RegistrationError::InvalidButton { id: 1, .. })
        ));
    }
}
