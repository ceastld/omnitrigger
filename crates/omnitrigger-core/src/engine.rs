//! 事件匹配、长按与拖拽计时、消费及回退状态。

use std::collections::{BTreeMap, BTreeSet};

#[path = "retained.rs"]
mod retained;
use retained::RetainedSession;

use crate::model::direction;
use crate::registration::validate;
use crate::{
    Activation, ActiveDrag, Axis, Button, ContextError, Decision, DragEvent, DragPhase, InputEvent,
    InputKind, RegistrationError, ScreenRect, TriggerSpec, WheelDirection, WheelPolicy, regions,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PressState {
    Pending,
    /// 层内等待中的按压，没有交给应用，也不会在短按或取消时回放。
    PendingSuppressed,
    Consumed,
    Passed,
    Draining,
    /// 已立即交给应用、但仍算作组合前缀的按下；释放同样交给应用，不消费也不回放。
    Observed,
}

/// 左键是所有界面操作的主键（点击、拖动标题栏、框选、拖放），作为前缀时不扣留，
/// 否则在组合未完成前所有程序都收不到按下，拖动会全部失效。
const PRIMARY: Button = Button::Mouse(crate::MouseButton::Left);

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

#[derive(Clone, Copy)]
struct PendingTapRestore {
    button: Button,
    deadline_ms: Option<u64>,
}

/// 默认组合要求当前按下集合与 held 完全一致；捕获集合模式须显式启用。
pub struct Engine {
    bindings: Vec<TriggerSpec>,
    pressed: BTreeMap<Button, PressState>,
    /// 一次物理按压在认领为末键后，直到释放都不能借给其他组合作前缀。
    terminal_presses: BTreeSet<Button>,
    captured_presses: BTreeMap<Button, usize>,
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
    /// 首个等待中的鼠标前缀按下时的指针位置；移出容差后放弃组合并回放，交还拖拽。
    prefix_origin: Option<(i32, i32)>,
    screens: Vec<ScreenRect>,
    process_name: Option<String>,
    window_role: crate::WindowRole,
    active_bindings: Vec<usize>,
    requires_process: Vec<bool>,
    requires_role: Vec<bool>,
    dedicated_leader: Option<Button>,
    repeat_owner: BTreeMap<Button, u64>,
    tap_restore: Option<PendingTapRestore>,
    layer_owner: Option<usize>,
    tap_allowed: Vec<bool>,
    retained: Option<RetainedSession>,
    last_keyboard_down: Option<u64>,
    retained_starters: Vec<usize>,
}

impl Engine {
    pub fn new(bindings: Vec<TriggerSpec>) -> Result<Self, RegistrationError> {
        let mut engine = Self {
            bindings: validate(bindings)?,
            pressed: BTreeMap::new(),
            terminal_presses: BTreeSet::new(),
            captured_presses: BTreeMap::new(),
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
            prefix_origin: None,
            screens: Vec::new(),
            process_name: None,
            window_role: crate::WindowRole::Unknown,
            active_bindings: Vec::new(),
            requires_process: Vec::new(),
            requires_role: Vec::new(),
            dedicated_leader: None,
            repeat_owner: BTreeMap::new(),
            tap_restore: None,
            layer_owner: None,
            tap_allowed: Vec::new(),
            retained: None,
            last_keyboard_down: None,
            retained_starters: Vec::new(),
        };
        engine.compile_process_scopes();
        Ok(engine)
    }
    pub fn replace_bindings(
        &mut self,
        bindings: Vec<TriggerSpec>,
    ) -> Result<Decision, RegistrationError> {
        let bindings = validate(bindings)?;
        let decision = self.cancel();
        self.bindings = bindings;
        self.compile_process_scopes();
        Ok(decision)
    }

    /// 映像文件名由平台显式提供。未知时不匹配有范围的规则，也不猜测其全局回退。
    /// 无效名称保留旧上下文；实际变化取消待定输入、计时和拖拽，并消费旧按压的释放。
    pub fn set_foreground_process(
        &mut self,
        name: Option<&str>,
    ) -> Result<Decision, crate::ProcessScopeError> {
        self.set_foreground_context(name, crate::WindowRole::Unknown)
    }

    pub fn set_foreground_context(
        &mut self,
        name: Option<&str>,
        role: crate::WindowRole,
    ) -> Result<Decision, crate::ProcessScopeError> {
        let normalized = name
            .map(|name| {
                crate::process_scope::normalize_name(name).ok_or_else(|| {
                    crate::ProcessScopeError::InvalidName {
                        name: name.to_owned(),
                    }
                })
            })
            .transpose()?;
        if self.process_name == normalized && self.window_role == role {
            return Ok(Decision::default());
        }
        let decision = self.cancel();
        self.process_name = normalized;
        self.window_role = role;
        self.refresh_active_bindings();
        Ok(decision)
    }

    pub fn needs_process_context(&self) -> bool {
        self.bindings.iter().any(|binding| {
            !binding.process_scope.is_global()
                || !binding.keyboard.tap_scope.is_global()
                || binding
                    .keyboard
                    .retained_prefix
                    .as_ref()
                    .is_some_and(|policy| !policy.capture_scope.is_global())
        })
    }

    pub fn foreground_process(&self) -> Option<&str> {
        self.process_name.as_deref()
    }

    pub fn foreground_window_role(&self) -> crate::WindowRole {
        self.window_role
    }

    pub fn needs_shell_context(&self) -> bool {
        self.bindings.iter().any(|binding| {
            binding.process_scope.needs_role()
                || binding.keyboard.tap_scope.needs_role()
                || binding
                    .keyboard
                    .retained_prefix
                    .as_ref()
                    .is_some_and(|policy| policy.capture_scope.needs_role())
        })
    }

    fn compile_process_scopes(&mut self) {
        self.requires_role = self
            .bindings
            .iter()
            .map(|binding| {
                self.bindings.iter().any(|other| {
                    other.process_scope.needs_role()
                        && (if binding.keyboard.matching.is_captured() {
                            other.keyboard.matching.is_captured()
                                && binding.keyboard.prefix() == other.keyboard.prefix()
                        } else {
                            other.held == binding.held
                                && same_activation_family(other.activation, binding.activation)
                        })
                })
            })
            .collect();
        self.requires_process = self
            .bindings
            .iter()
            .map(|binding| {
                binding.process_scope.is_global()
                    && self.bindings.iter().any(|other| {
                        other.process_scope.priority() > 0
                            && (if binding.keyboard.matching.is_captured() {
                                other.keyboard.matching.is_captured()
                                    && binding.keyboard.prefix() == other.keyboard.prefix()
                            } else {
                                other.held == binding.held
                                    && same_activation_family(other.activation, binding.activation)
                            })
                    })
            })
            .collect();
        self.refresh_active_bindings();
    }

    fn refresh_active_bindings(&mut self) {
        self.retained_starters = self
            .bindings
            .iter()
            .enumerate()
            .filter_map(|(index, binding)| {
                binding
                    .keyboard
                    .retained_prefix
                    .as_ref()
                    .filter(|policy| {
                        policy
                            .capture_scope
                            .matches_context(self.process_name.as_deref(), self.window_role)
                    })
                    .map(|_| index)
            })
            .collect();
        self.tap_allowed = self
            .bindings
            .iter()
            .map(|binding| {
                binding
                    .keyboard
                    .tap_scope
                    .matches_context(self.process_name.as_deref(), self.window_role)
            })
            .collect();
        self.active_bindings = self
            .bindings
            .iter()
            .enumerate()
            .filter_map(|(index, binding)| {
                (binding
                    .process_scope
                    .matches_context(self.process_name.as_deref(), self.window_role)
                    && (self.process_name.is_some() || !self.requires_process[index])
                    && (self.window_role != crate::WindowRole::Unknown
                        || !self.requires_role[index]))
                    .then_some(index)
            })
            .collect();
        self.active_bindings.sort_by_key(|&index| {
            let binding = &self.bindings[index];
            (
                std::cmp::Reverse(binding.process_scope.priority()),
                binding.keyboard.matching.priority(),
            )
        });
    }

    fn candidates(&self) -> impl Iterator<Item = &TriggerSpec> {
        self.active_bindings
            .iter()
            .map(|&index| &self.bindings[index])
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
        self.dedicated_leader = None;
        self.layer_owner = None;
        self.repeat_owner.clear();
        self.terminal_presses.clear();
        self.captured_presses.clear();
        self.tap_restore = None;
        self.retained = None;
        self.pending.clear();
        self.prefix_origin = None;
        self.hold = None;
        self.drag = None;
        self.wheel = [0; 4];
        self.wheel_owner = [None; 4];
        for state in self.pressed.values_mut() {
            if matches!(
                *state,
                PressState::Pending | PressState::PendingSuppressed | PressState::Consumed
            ) {
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
        if self.tap_restore.is_some_and(|tap| matches!(event.kind,
            InputKind::Down(Button::Key(_)) | InputKind::Up(Button::Key(_)))
            && !matches!(event.kind, InputKind::Down(button) | InputKind::Up(button) if button == tap.button))
        {
            self.tap_restore = None;
        }
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
        let current = if let Some(decision) = self.retained_event(event.kind) {
            decision
        } else {
            match event.kind {
                InputKind::Down(button) => self.down(button),
                InputKind::Up(button) => self.up(button),
                InputKind::Wheel { axis, delta } => self.scroll(axis, delta),
            }
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
        if self.tap_restore.is_some_and(|tap| {
            tap.deadline_ms
                .is_some_and(|deadline| self.now_ms >= deadline)
        }) {
            self.tap_restore = None;
        }
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
                    beyond_tolerance(origin, position)
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
        // 鼠标键作为普通前缀等待时，移动说明用户在拖拽（如拖动标题栏、框选）：
        // 放弃组合并立即回放按下，让应用收到真实的按下与后续移动，而不是松开时的一次点击。
        if self.hold.is_none()
            && self.drag.is_none()
            && self.active_drag.is_none()
            && self.pressed.iter().any(|(button, state)| {
                matches!(button, Button::Mouse(_)) && *state == PressState::Pending
            })
        {
            let origin = *self.prefix_origin.get_or_insert(position);
            if beyond_tolerance(origin, position) {
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
        if let Some(state) = self.pressed.get(&button).copied() {
            // 自动重复不是第二次按压；已消费的按压也不能通过重复事件泄漏。
            return Decision {
                consume: !matches!(state, PressState::Passed | PressState::Observed),
                triggered: if state == PressState::Consumed {
                    self.repeated_action(button).into_iter().collect()
                } else {
                    Vec::new()
                },
                ..Decision::default()
            };
        }
        if !self.enabled {
            self.pressed.insert(button, PressState::Passed);
            return Decision::default();
        }
        let priority = self
            .candidates()
            .find(|binding| {
                same_activation_family(binding.activation, Activation::Press(button))
                    && self.binding_held_matches(binding)
                    && self.conditions_match(binding)
            })
            .map(|binding| binding.process_scope.priority());
        let press = self
            .candidates()
            .find(|binding| {
                binding.activation == Activation::Press(button)
                    && Some(binding.process_scope.priority()) == priority
                    && self.binding_held_matches(binding)
                    && self.conditions_match(binding)
            })
            .map(|binding| (binding.id, binding.keyboard.repeat));
        if let Some((id, repeat)) = press {
            self.commit_pending();
            self.pressed.insert(button, PressState::Consumed);
            self.terminal_presses.insert(button);
            if repeat {
                self.repeat_owner.insert(button, id);
            }
            return Decision {
                consume: true,
                triggered: vec![id],
                ..Decision::default()
            };
        }
        let hold = self
            .candidates()
            .find_map(|binding| match binding.activation {
                Activation::Hold {
                    button: terminal,
                    duration_ms,
                } if terminal == button
                    && Some(binding.process_scope.priority()) == priority
                    && self.binding_held_matches(binding)
                    && self.conditions_match(binding) =>
                {
                    Some((binding.id, duration_ms))
                }
                _ => None,
            });
        let drag = self
            .candidates()
            .find_map(|binding| match binding.activation {
                Activation::Drag {
                    button: terminal,
                    min_distance_px,
                    max_start_ms,
                } if terminal == button
                    && Some(binding.process_scope.priority()) == priority
                    && self.binding_held_matches(binding)
                    && self.conditions_match(binding) =>
                {
                    Some((binding.id, min_distance_px, max_start_ms))
                }
                _ => None,
            });
        if hold.is_some() || drag.is_some() {
            self.terminal_presses.insert(button);
            self.wheel = [0; 4];
            self.wheel_owner = [None; 4];
            let suppressed = self.dedicated_leader.is_some() && matches!(button, Button::Key(_));
            self.pressed.insert(
                button,
                if suppressed {
                    PressState::PendingSuppressed
                } else {
                    PressState::Pending
                },
            );
            if !suppressed {
                self.pending.push(InputKind::Down(button));
            }
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
        let eligible = self.terminal_presses.is_empty()
            && self.pressed.values().all(|state| {
                matches!(
                    state,
                    PressState::Pending
                        | PressState::PendingSuppressed
                        | PressState::Consumed
                        | PressState::Observed
                )
            });
        let extends_prefix = if self.captured_layer().is_some() {
            self.captured_extension(button).is_some()
        } else {
            eligible
                && self.candidates().any(|binding| {
                    binding.keyboard.retained_prefix.is_none()
                        && binding.held.contains(&button)
                        && self.conditions_match(binding)
                        && self
                            .pressed
                            .keys()
                            .all(|pressed| binding.held.contains(pressed))
                })
        };
        if extends_prefix && button == PRIMARY {
            // 只记录为前缀，按下照常交给应用；其他等待中的前缀保持不变。
            self.wheel = [0; 4];
            self.wheel_owner = [None; 4];
            self.pressed.insert(button, PressState::Observed);
            return Decision::default();
        }
        if extends_prefix {
            self.wheel = [0; 4];
            self.wheel_owner = [None; 4];
            if matches!(button, Button::Mouse(_)) && self.prefix_origin.is_none() {
                self.prefix_origin = self.pointer_position;
            }
            if let Some(index) = self
                .dedicated_index(button)
                .filter(|_| self.dedicated_leader.is_none())
            {
                let isolated = self.pressed.is_empty();
                let restore =
                    self.bindings[index].keyboard.restore_on_tap && self.tap_allowed[index];
                let timeout = self.bindings[index].keyboard.tap_timeout_ms;
                self.commit_pending();
                self.dedicated_leader = Some(button);
                self.layer_owner = Some(index);
                if isolated && restore {
                    self.tap_restore = Some(PendingTapRestore {
                        button,
                        deadline_ms: timeout.map(|ms| self.now_ms.saturating_add(u64::from(ms))),
                    });
                }
            }
            if self.dedicated_leader.is_some() && matches!(button, Button::Key(_)) {
                if let Some(index) = self.captured_extension(button) {
                    self.captured_presses.insert(button, index);
                }
                self.pressed.insert(button, PressState::Consumed);
            } else {
                self.pressed.insert(button, PressState::Pending);
                self.pending.push(InputKind::Down(button));
            }
            return Decision {
                consume: true,
                ..Decision::default()
            };
        }
        let replay = self.flush_pending();
        if matches!(button, Button::Key(_)) && self.dedicated_leader.is_some() {
            self.pressed.insert(button, PressState::Consumed);
            return Decision {
                consume: true,
                replay,
                ..Decision::default()
            };
        }
        if self.is_dedicated_prefix(button) {
            // 不能借用此前已交付给应用的按压来启动层；仍抑制专用键自身。
            self.pressed.insert(button, PressState::Draining);
            return Decision {
                consume: true,
                replay,
                ..Decision::default()
            };
        }
        self.pressed.insert(button, PressState::Passed);
        Decision {
            replay,
            ..Decision::default()
        }
    }

    fn up(&mut self, button: Button) -> Decision {
        self.repeat_owner.remove(&button);
        self.terminal_presses.remove(&button);
        self.captured_presses.remove(&button);
        let leaving_layer = self.dedicated_leader == Some(button);
        let restore_tap = leaving_layer && self.tap_restore.is_some_and(|tap| tap.button == button);
        if leaving_layer {
            self.dedicated_leader = None;
            self.layer_owner = None;
            self.repeat_owner.clear();
            self.captured_presses.clear();
            self.tap_restore = None;
        }
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
        if leaving_layer {
            for state in self.pressed.values_mut() {
                if matches!(*state, PressState::Consumed | PressState::PendingSuppressed) {
                    *state = PressState::Draining;
                }
            }
        }
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
            PressState::Consumed | PressState::PendingSuppressed => {
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
            PressState::Passed | PressState::Observed => Decision::default(),
        };
        if restore_tap {
            decision
                .replay
                .extend([InputKind::Down(button), InputKind::Up(button)]);
        }
        decision.drags.extend(drag);
        decision
    }

    fn scroll(&mut self, axis: Axis, delta: i32) -> Decision {
        if delta == 0 || !self.enabled {
            return Decision::default();
        }
        let direction = direction(axis, delta);
        let Some((id, policy)) = self
            .candidates()
            .find(|binding| {
                binding.activation == Activation::Wheel(direction)
                    && self.binding_held_matches(binding)
                    && self.conditions_match(binding)
            })
            .map(|binding| (binding.id, binding.wheel))
        else {
            return Decision {
                replay: self.flush_pending(),
                ..Decision::default()
            };
        };
        let index = wheel_index(direction);
        self.tap_restore = None;
        if self.wheel_owner[index] != Some(id) {
            self.wheel[index] = 0;
        }
        self.wheel_owner[index] = Some(id);
        let count = match policy {
            WheelPolicy::Packet => {
                self.wheel[index] = 0;
                1
            }
            WheelPolicy::Notch => {
                self.wheel[index] = self.wheel[index].saturating_add(i64::from(delta).abs());
                // 一条异常大的公共API输入不能让hook同步路径产生无界工作；剩余步数保留。
                let count = (self.wheel[index] / 120).min(64) as usize;
                self.wheel[index] -= count as i64 * 120;
                count
            }
        };
        if count > 0 {
            self.commit_pending();
            self.append_regional_tails();
        } else if self.binding_keeps_wheel_remainder(id)
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

    fn is_dedicated_prefix(&self, button: Button) -> bool {
        self.candidates().any(|binding| {
            binding.keyboard.dedicated_prefix == Some(button) && self.conditions_match(binding)
        })
    }

    fn dedicated_index(&self, button: Button) -> Option<usize> {
        self.retained_starters
            .iter()
            .copied()
            .chain(
                self.active_bindings
                    .iter()
                    .copied()
                    .filter(|&index| self.bindings[index].keyboard.retained_prefix.is_none()),
            )
            .find(|&index| {
                let binding = &self.bindings[index];
                binding.keyboard.prefix() == Some(button) && self.conditions_match(binding)
            })
    }

    fn repeated_action(&self, button: Button) -> Option<u64> {
        if self.captured_layer().is_some_and(|leader| leader != button) {
            // 捕获模式每次重复都按当前捕获集合重新选择，已捕获附加键不会因重复丢失角色。
            return self
                .candidates()
                .find(|binding| {
                    binding.activation == Activation::Press(button)
                        && self.binding_held_matches(binding)
                        && self.conditions_match(binding)
                })
                .filter(|binding| binding.keyboard.repeat)
                .map(|binding| binding.id);
        }
        let id = *self.repeat_owner.get(&button)?;
        self.candidates()
            .find(|binding| {
                binding.id == id
                    && self.conditions_match(binding)
                    && (if binding.keyboard.matching.is_captured() {
                        self.binding_held_matches(binding)
                    } else {
                        self.pressed.len() == binding.held.len() + 1
                            && self.pressed.iter().all(|(&pressed, &state)| {
                                pressed == button
                                    || (binding.held.contains(&pressed)
                                        && !self.terminal_presses.contains(&pressed)
                                        && matches!(
                                            state,
                                            PressState::Pending
                                                | PressState::PendingSuppressed
                                                | PressState::Consumed
                                                | PressState::Observed
                                        ))
                            })
                    })
            })
            .map(|binding| binding.id)
    }

    fn held_matches(&self, held: &[Button]) -> bool {
        self.pressed.len() == held.len()
            && self.pressed.iter().all(|(button, state)| {
                matches!(
                    state,
                    PressState::Pending
                        | PressState::PendingSuppressed
                        | PressState::Consumed
                        | PressState::Observed
                ) && held.contains(button)
                    && !self.terminal_presses.contains(button)
            })
    }

    fn captured_layer(&self) -> Option<Button> {
        self.dedicated_leader.filter(|_| {
            self.layer_owner
                .is_some_and(|index| self.bindings[index].keyboard.matching.is_captured())
        })
    }

    fn captured_extension(&self, button: Button) -> Option<usize> {
        let leader = self.captured_layer()?;
        if button == leader {
            return None;
        }
        self.active_bindings.iter().copied().find(|&index| {
            let binding = &self.bindings[index];
            binding.keyboard.matching.is_captured()
                && binding.keyboard.prefix() == Some(leader)
                && binding.held.contains(&button)
                && self.conditions_match(binding)
        })
    }

    fn binding_held_matches(&self, binding: &TriggerSpec) -> bool {
        if !binding.keyboard.matching.is_captured() {
            return self.held_matches(&binding.held);
        }
        let Some(leader) = self.captured_layer() else {
            return false;
        };
        binding.keyboard.prefix() == Some(leader)
            && (self.pressed.get(&leader) == Some(&PressState::Consumed)
                || (self.retained.is_some()
                    && self.pressed.get(&leader) == Some(&PressState::Observed)))
            && binding
                .held
                .iter()
                .all(|key| *key == leader || self.captured_presses.contains_key(key))
    }

    fn conditions_match(&self, binding: &TriggerSpec) -> bool {
        binding
            .conditions
            .iter()
            .all(|condition| condition.matches(self.pointer_position, &self.screens))
    }

    fn binding_keeps_wheel_remainder(&self, id: u64) -> bool {
        self.candidates().any(|binding| {
            binding.id == id
                && (!binding.conditions.is_empty() || binding.keyboard.prefix().is_some())
        })
    }

    fn binding_conditions_match(&self, id: u64) -> bool {
        self.candidates()
            .find(|binding| binding.id == id)
            .is_some_and(|binding| self.conditions_match(binding))
    }

    fn reconcile_conditions(&mut self) -> Decision {
        let lost_captures = self
            .captured_presses
            .iter()
            .filter_map(|(&key, &index)| {
                (!self.conditions_match(&self.bindings[index])).then_some(key)
            })
            .collect::<Vec<_>>();
        for key in lost_captures {
            self.captured_presses.remove(&key);
            if let Some(state) = self.pressed.get_mut(&key) {
                *state = PressState::Draining;
            }
        }
        if self.dedicated_leader.is_some_and(|button| {
            self.layer_owner
                .zip(self.dedicated_index(button))
                .is_none_or(|(owner, current)| {
                    !self.bindings[owner]
                        .keyboard
                        .same_layer(&self.bindings[current].keyboard)
                })
        }) {
            let replay = self.flush_pending();
            let mut decision = self.cancel();
            decision.replay = replay;
            return decision;
        }
        let revoked = self
            .repeat_owner
            .iter()
            .filter_map(|(&button, &id)| (!self.binding_conditions_match(id)).then_some(button))
            .collect::<Vec<_>>();
        for button in revoked {
            self.repeat_owner.remove(&button);
        }
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
            && !self.candidates().any(|binding| {
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
                || !self.wheel_owner[index].is_some_and(|id| self.binding_keeps_wheel_remainder(id))
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
        self.tap_restore = None;
        self.prefix_origin = None;
        self.pending.clear();
        self.hold = None;
        self.drag = None;
        for state in self.pressed.values_mut() {
            if matches!(*state, PressState::Pending | PressState::PendingSuppressed) {
                *state = PressState::Consumed;
            }
        }
    }

    fn flush_pending(&mut self) -> Vec<InputKind> {
        self.prefix_origin = None;
        self.hold = None;
        self.drag = None;
        self.wheel = [0; 4];
        self.wheel_owner = [None; 4];
        for state in self.pressed.values_mut() {
            if *state == PressState::Pending {
                *state = PressState::Passed;
            } else if *state == PressState::PendingSuppressed {
                *state = PressState::Consumed;
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

fn same_activation_family(left: Activation, right: Activation) -> bool {
    let button = |activation| match activation {
        Activation::Press(button)
        | Activation::Hold { button, .. }
        | Activation::Drag { button, .. } => Some(button),
        Activation::Wheel(_) => None,
    };
    match (button(left), button(right)) {
        (Some(left), Some(right)) => left == right,
        (None, None) => left == right,
        _ => false,
    }
}

/// 鼠标长按与鼠标前缀共用的静止容差：相对按下点 8 个物理像素的半径。
fn beyond_tolerance(origin: (i32, i32), position: (i32, i32)) -> bool {
    let dx = i64::from(position.0) - i64::from(origin.0);
    let dy = i64::from(position.1) - i64::from(origin.1);
    // 先限制各轴，平方运算只处理 -8..=8，极端坐标也不会溢出。
    dx.abs() > 8 || dy.abs() > 8 || dx * dx + dy * dy > 64
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
