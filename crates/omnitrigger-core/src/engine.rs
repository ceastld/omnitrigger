//! 事件匹配、长按与拖拽计时、消费及回退状态。

use std::collections::BTreeMap;

use crate::model::direction;
use crate::registration::validate;
use crate::{
    Activation, ActiveDrag, Axis, Button, ContextError, Decision, DragEvent, DragPhase, InputEvent,
    InputKind, RegistrationError, ScreenRect, TriggerSpec, WheelDirection, regions,
};

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

#[derive(Clone, Copy)]
struct PendingDrag {
    id: u64,
    button: Button,
    deadline_ms: u64,
    min_distance_px: u32,
    pointer_origin: Option<(i32, i32)>,
}

/// 每个组合要求当前按下集合与 held 完全一致；多余按键不会被静默忽略。
pub struct Engine {
    bindings: Vec<TriggerSpec>,
    pressed: BTreeMap<Button, PressState>,
    pending: Vec<InputKind>,
    wheel: [i64; 4],
    wheel_owner: [Option<u64>; 4],
    enabled: bool,
    now_ms: u64,
    hold: Option<PendingHold>,
    drag: Option<PendingDrag>,
    active_drag: Option<ActiveDrag>,
    next_drag_session: Option<u64>,
    pointer_position: Option<(i32, i32)>,
    screens: Vec<ScreenRect>,
}

impl Engine {
    pub fn new(bindings: Vec<TriggerSpec>) -> Result<Self, RegistrationError> {
        Ok(Self {
            bindings: validate(bindings)?,
            pressed: BTreeMap::new(),
            pending: Vec::new(),
            wheel: [0; 4],
            wheel_owner: [None; 4],
            enabled: true,
            now_ms: 0,
            hold: None,
            drag: None,
            active_drag: None,
            next_drag_session: Some(1),
            pointer_position: None,
            screens: Vec::new(),
        })
    }
    pub fn replace_bindings(
        &mut self,
        bindings: Vec<TriggerSpec>,
    ) -> Result<Decision, RegistrationError> {
        let bindings = validate(bindings)?;
        let decision = self.cancel();
        self.bindings = bindings;
        Ok(decision)
    }

    /// 屏幕拓扑来自平台显式快照。无效替换不改变状态；实际变更取消旧上下文。
    pub fn set_screens(&mut self, screens: Vec<ScreenRect>) -> Result<Decision, ContextError> {
        regions::validate_screens(&screens)?;
        if self.screens == screens {
            return Ok(Decision::default());
        }
        let decision = self.cancel();
        self.screens = screens;
        Ok(decision)
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Decision {
        if self.enabled != enabled {
            let decision = self.cancel();
            self.enabled = enabled;
            decision
        } else {
            Decision::default()
        }
    }
    /// 取消上下文并消费旧按压的后续释放；活动拖拽的终止事件与收尾决策同步返回。
    pub fn cancel(&mut self) -> Decision {
        let drag = self.finish_drag(DragPhase::Cancelled);
        self.pending.clear();
        self.hold = None;
        self.drag = None;
        self.wheel = [0; 4];
        self.wheel_owner = [None; 4];
        for state in self.pressed.values_mut() {
            if matches!(*state, PressState::Pending | PressState::Consumed) {
                *state = PressState::Draining;
            }
        }
        Decision {
            drags: drag.into_iter().collect(),
            ..Decision::default()
        }
    }

    /// 过期的界面取消请求不能终止之后开始的新拖拽会话。
    pub fn cancel_drag(&mut self, session: u64) -> Decision {
        if self.active_drag.is_some_and(|drag| drag.session == session) {
            self.cancel()
        } else {
            Decision::default()
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
        let interrupts_press = match event.kind {
            InputKind::Down(button) => !self.pressed.contains_key(&button),
            // 释放由 up 按原 Pending/Consumed 语义回放并收尾，避免先转 Passed。
            InputKind::Up(_) => false,
            InputKind::Wheel { delta, .. } => delta != 0,
        };
        if interrupts_press && (self.hold.is_some() || self.drag.is_some()) {
            decision.replay.extend(self.flush_pending());
        }
        if interrupts_press {
            decision
                .drags
                .extend(self.finish_drag(DragPhase::Cancelled));
        }
        let current = match event.kind {
            InputKind::Down(button) => self.down(button),
            InputKind::Up(button) => self.up(button),
            InputKind::Wheel { axis, delta } => self.scroll(axis, delta),
        };
        decision.consume = current.consume;
        decision.replay.extend(current.replay);
        decision.triggered.extend(current.triggered);
        decision.drags.extend(current.drags);
        decision
    }

    /// 时间单调前进；倒退的时间戳按最近时间处理，避免重复触发或延长等待。
    pub fn advance_time(&mut self, now_ms: u64) -> Decision {
        let mut decision = self.reconcile_conditions();
        self.now_ms = self.now_ms.max(now_ms);
        if self
            .drag
            .is_some_and(|drag| self.now_ms >= drag.deadline_ms)
        {
            self.drag = None;
            // 拖拽窗口过期不取消仍在等待的长按；长按已因移动取消时才回放。
            if self.hold.is_none() {
                decision.replay.extend(self.flush_pending());
            }
        }
        let Some(hold) = self.hold else {
            return decision;
        };
        if self.now_ms < hold.deadline_ms {
            return decision;
        }
        self.commit_pending();
        decision.triggered.push(hold.id);
        decision
    }

    pub fn next_deadline_ms(&self) -> Option<u64> {
        [
            self.hold.map(|hold| hold.deadline_ms),
            self.drag.map(|drag| drag.deadline_ms),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    pub fn next_hold_deadline_ms(&self) -> Option<u64> {
        self.hold.map(|hold| hold.deadline_ms)
    }

    pub fn pending_drag_id(&self) -> Option<u64> {
        self.drag.map(|drag| drag.id)
    }

    pub fn active_drag(&self) -> Option<ActiveDrag> {
        self.active_drag
    }

    /// 只传入物理鼠标位置；平台应在鼠标按下前同步位置，供长按记录起点。
    /// 鼠标长按在 8 像素半径外取消；仍有拖拽候选时继续等待，不提前回放。
    /// 先推进截止时间，拖拽仅接受严格早于 max_start_ms 的位移；键盘长按不受移动影响。
    pub fn pointer_moved(&mut self, position: (i32, i32), now_ms: u64) -> Decision {
        self.pointer_position = Some(position);
        if let Some(drag) = &mut self.active_drag {
            drag.position = position;
        }
        // 区域失效不能在本次移动恰好到达截止时间时仍误触发；无条件计时顺序保持不变。
        let mut decision = self.advance_time(now_ms);
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
            self.hold = None;
            if self.drag.is_none() {
                decision.replay.extend(self.flush_pending());
            }
        }
        if let Some(drag) = &mut self.drag {
            let origin = *drag.pointer_origin.get_or_insert(position);
            if distance_at_least(origin, position, drag.min_distance_px) {
                let Some(session) = self.next_drag_session else {
                    // 会话编号耗尽后不再认领新的拖拽，不复用编号也不丢失原按下。
                    self.drag = None;
                    if self.hold.is_none() {
                        decision.replay.extend(self.flush_pending());
                    }
                    return decision;
                };
                self.next_drag_session = session.checked_add(1);
                let active = ActiveDrag {
                    id: drag.id,
                    session,
                    button: drag.button,
                    origin,
                    position,
                };
                self.commit_pending();
                self.active_drag = Some(active);
                decision.triggered.push(active.id);
                decision.drags.push(DragEvent {
                    id: active.id,
                    session: active.session,
                    phase: DragPhase::Started,
                    origin,
                    position,
                });
            }
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
                binding.activation == Activation::Press(button)
                    && self.held_matches(&binding.held)
                    && self.conditions_match(binding)
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
        let hold = self
            .bindings
            .iter()
            .find_map(|binding| match binding.activation {
                Activation::Hold {
                    button: terminal,
                    duration_ms,
                } if terminal == button
                    && self.held_matches(&binding.held)
                    && self.conditions_match(binding) =>
                {
                    Some((binding.id, duration_ms))
                }
                _ => None,
            });
        let drag = self
            .bindings
            .iter()
            .find_map(|binding| match binding.activation {
                Activation::Drag {
                    button: terminal,
                    min_distance_px,
                    max_start_ms,
                } if terminal == button
                    && self.held_matches(&binding.held)
                    && self.conditions_match(binding) =>
                {
                    Some((binding.id, min_distance_px, max_start_ms))
                }
                _ => None,
            });
        if hold.is_some() || drag.is_some() {
            self.wheel = [0; 4];
            self.wheel_owner = [None; 4];
            self.pressed.insert(button, PressState::Pending);
            self.pending.push(InputKind::Down(button));
            self.hold = hold.map(|(id, duration_ms)| PendingHold {
                id,
                button,
                deadline_ms: self.now_ms.saturating_add(u64::from(duration_ms)),
                pointer_origin: self.pointer_position,
            });
            self.drag = drag.map(|(id, min_distance_px, max_start_ms)| PendingDrag {
                id,
                button,
                deadline_ms: self.now_ms.saturating_add(u64::from(max_start_ms)),
                min_distance_px,
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
                    && self.conditions_match(binding)
                    && self
                        .pressed
                        .keys()
                        .all(|pressed| binding.held.contains(pressed))
            });
        if extends_prefix {
            self.wheel = [0; 4];
            self.wheel_owner = [None; 4];
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
        let drag = if self.pressed.contains_key(&button) {
            self.active_drag.and_then(|drag| {
                self.finish_drag(if drag.button == button {
                    DragPhase::Ended
                } else {
                    DragPhase::Cancelled
                })
            })
        } else {
            None
        };
        let Some(state) = self.pressed.remove(&button) else {
            return Decision::default();
        };
        self.wheel = [0; 4];
        self.wheel_owner = [None; 4];
        let mut decision = match state {
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
        };
        decision.drags.extend(drag);
        decision
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
                    && self.conditions_match(binding)
            })
            .map(|binding| binding.id)
        else {
            return Decision {
                replay: self.flush_pending(),
                ..Decision::default()
            };
        };
        let index = wheel_index(direction);
        if self.wheel_owner[index] != Some(id) {
            self.wheel[index] = 0;
        }
        self.wheel_owner[index] = Some(id);
        self.wheel[index] = self.wheel[index].saturating_add(i64::from(delta).abs());
        // 一条异常大的公共API输入不能让hook同步路径产生无界工作；剩余步数保留。
        let count = (self.wheel[index] / 120).min(64) as usize;
        self.wheel[index] -= count as i64 * 120;
        if count > 0 {
            self.commit_pending();
            self.append_regional_tails();
        } else if self.binding_has_conditions(id)
            || self
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

    fn conditions_match(&self, binding: &TriggerSpec) -> bool {
        binding
            .conditions
            .iter()
            .all(|condition| condition.matches(self.pointer_position, &self.screens))
    }

    fn binding_has_conditions(&self, id: u64) -> bool {
        self.bindings
            .iter()
            .any(|binding| binding.id == id && !binding.conditions.is_empty())
    }

    fn binding_conditions_match(&self, id: u64) -> bool {
        self.bindings
            .iter()
            .find(|binding| binding.id == id)
            .is_some_and(|binding| self.conditions_match(binding))
    }

    fn reconcile_conditions(&mut self) -> Decision {
        let mut decision = Decision::default();
        let had_timed = self.hold.is_some() || self.drag.is_some();
        if self
            .hold
            .is_some_and(|hold| !self.binding_conditions_match(hold.id))
        {
            self.hold = None;
        }
        if self
            .drag
            .is_some_and(|drag| !self.binding_conditions_match(drag.id))
        {
            self.drag = None;
        }
        if self
            .active_drag
            .is_some_and(|drag| !self.binding_conditions_match(drag.id))
        {
            decision
                .drags
                .extend(self.finish_drag(DragPhase::Cancelled));
        }
        let lost_wheel = self.wheel_owner.iter().enumerate().any(|(index, owner)| {
            self.wheel[index] != 0 && owner.is_some_and(|id| !self.binding_conditions_match(id))
        });
        let lost_prefix = self.hold.is_none()
            && self.drag.is_none()
            && self
                .pressed
                .values()
                .any(|state| *state == PressState::Pending)
            && !self.bindings.iter().any(|binding| {
                self.conditions_match(binding)
                    && self
                        .pressed
                        .keys()
                        .all(|button| binding.held.contains(button))
            });
        if lost_wheel || lost_prefix || (had_timed && self.hold.is_none() && self.drag.is_none()) {
            decision.replay.extend(self.flush_pending());
        }
        decision
    }

    fn append_regional_tails(&mut self) {
        for index in 0..4 {
            if self.wheel[index] == 0
                || !self.wheel_owner[index].is_some_and(|id| self.binding_has_conditions(id))
            {
                continue;
            }
            // 正常高精度余量小于一格；异常公共输入的补发同样保持固定工作量。
            let magnitude = self.wheel[index].min(i64::from(i32::MAX)) as i32;
            let (axis, delta) = match index {
                0 => (Axis::Vertical, magnitude),
                1 => (Axis::Vertical, -magnitude),
                2 => (Axis::Horizontal, -magnitude),
                _ => (Axis::Horizontal, magnitude),
            };
            self.pending.push(InputKind::Wheel { axis, delta });
        }
    }

    fn commit_pending(&mut self) {
        self.pending.clear();
        self.hold = None;
        self.drag = None;
        for state in self.pressed.values_mut() {
            if *state == PressState::Pending {
                *state = PressState::Consumed;
            }
        }
    }

    fn flush_pending(&mut self) -> Vec<InputKind> {
        self.hold = None;
        self.drag = None;
        self.wheel = [0; 4];
        self.wheel_owner = [None; 4];
        for state in self.pressed.values_mut() {
            if *state == PressState::Pending {
                *state = PressState::Passed;
            }
        }
        std::mem::take(&mut self.pending)
    }

    fn finish_drag(&mut self, phase: DragPhase) -> Option<DragEvent> {
        let drag = self.active_drag.take()?;
        if phase == DragPhase::Cancelled {
            for state in self.pressed.values_mut() {
                if *state == PressState::Consumed {
                    *state = PressState::Draining;
                }
            }
        }
        Some(DragEvent {
            id: drag.id,
            session: drag.session,
            phase,
            origin: drag.origin,
            position: drag.position,
        })
    }
}

fn distance_at_least(origin: (i32, i32), position: (i32, i32), distance: u32) -> bool {
    let dx = i64::from(position.0) - i64::from(origin.0);
    let dy = i64::from(position.1) - i64::from(origin.1);
    let distance = i64::from(distance);
    // 各轴先与有界阈值比较，只有小于500的差值才平方，极端屏幕坐标也不会溢出。
    dx.abs() >= distance || dy.abs() >= distance || dx * dx + dy * dy >= distance * distance
}

fn wheel_index(direction: WheelDirection) -> usize {
    match direction {
        WheelDirection::Up => 0,
        WheelDirection::Down => 1,
        WheelDirection::Left => 2,
        WheelDirection::Right => 3,
    }
}

#[cfg(test)]
#[path = "drag_tests.rs"]
mod drag_tests;
