//! 事件匹配、长按与拖拽计时、消费及回退状态。

use std::collections::{BTreeMap, BTreeSet};

#[path = "retained.rs"]
mod retained;
use retained::RetainedSession;

use crate::model::direction;
use crate::registration::validate;
use crate::{
    Activation, ActiveDrag, Axis, Button, ContextError, Decision, DragEvent, DragPhase,
    GestureTracker, InputEvent, InputKind, RegistrationError, ScreenRect, TriggerSpec,
    WheelDirection, WheelPolicy, WheelSessionSnapshot, regions,
};

const MOUSE_BUTTON_COUNT: usize = 5;
const SESSION_OWNER_WORDS: usize = crate::MAX_BINDINGS.div_ceil(64);

#[derive(Clone, Copy, Default)]
struct MousePrefixOwners {
    bindings: [u64; SESSION_OWNER_WORDS],
}

impl MousePrefixOwners {
    fn is_active(&self) -> bool {
        self.bindings.iter().any(|word| *word != 0)
    }

    fn insert(&mut self, index: usize) {
        self.bindings[index / 64] |= 1 << (index % 64);
    }

    fn contains(&self, index: usize) -> bool {
        self.bindings[index / 64] & (1 << (index % 64)) != 0
    }
}

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

/// 一次鼠标手势会话。候选规则在按下时确定，位置条件不随轨迹移动重新判定。
#[derive(Clone)]
struct GestureSession {
    button: Button,
    candidates: MousePrefixOwners,
    /// 按下时位置未知则在首次移动时以该点为起点。
    tracker: Option<GestureTracker>,
    deadline_ms: u64,
    /// 已认领本次按压：原按下不再回放，松开时识别。
    started: bool,
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
    /// 保留层仍物理按住的可恢复附加键，保留按下次序。
    retained_adorn_order: Vec<Button>,
    pending: Vec<InputKind>,
    wheel: [i64; 4],
    wheel_owner: [Option<u64>; 4],
    enabled: bool,
    now_ms: u64,
    hold: Option<PendingHold>,
    drag: Option<PendingDrag>,
    active_drag: Option<ActiveDrag>,
    gesture: Option<GestureSession>,
    next_drag_session: Option<u64>,
    pointer_position: Option<(i32, i32)>,
    /// 首个等待中的鼠标前缀按下时的指针位置；移出容差后放弃组合并回放，交还拖拽。
    prefix_origin: Option<(i32, i32)>,
    /// 仅在鼠标前缀实际扣留Down时建立；记录当时可用的规则，不借新范围重建旧会话。
    mouse_prefix_sessions: [MousePrefixOwners; MOUSE_BUTTON_COUNT],
    screens: Vec<ScreenRect>,
    process_name: Option<String>,
    window_role: crate::WindowRole,
    pointer_process: Option<String>,
    pointer_role: crate::WindowRole,
    active_bindings: Vec<usize>,
    requires_process: Vec<bool>,
    requires_role: Vec<bool>,
    requires_pointer_process: Vec<bool>,
    requires_pointer_role: Vec<bool>,
    dedicated_leader: Option<Button>,
    repeat_owner: BTreeMap<Button, u64>,
    tap_restore: Option<PendingTapRestore>,
    layer_owner: Option<usize>,
    tap_allowed: Vec<bool>,
    retained: Option<RetainedSession>,
    last_keyboard_down: Option<u64>,
    retained_starters: Vec<usize>,
    mouse_modifier_gate: Option<crate::MouseModifierGate>,
    mouse_modifier_context: Option<crate::MouseModifierContext>,
    uses_mouse_modifier_snapshot: bool,
}

impl Engine {
    pub fn new(bindings: Vec<TriggerSpec>) -> Result<Self, RegistrationError> {
        let mut engine = Self {
            bindings: validate(bindings)?,
            pressed: BTreeMap::new(),
            terminal_presses: BTreeSet::new(),
            captured_presses: BTreeMap::new(),
            retained_adorn_order: Vec::with_capacity(254),
            pending: Vec::new(),
            wheel: [0; 4],
            wheel_owner: [None; 4],
            enabled: true,
            now_ms: 0,
            hold: None,
            drag: None,
            active_drag: None,
            gesture: None,
            next_drag_session: Some(1),
            pointer_position: None,
            prefix_origin: None,
            mouse_prefix_sessions: [MousePrefixOwners::default(); MOUSE_BUTTON_COUNT],
            screens: Vec::new(),
            process_name: None,
            window_role: crate::WindowRole::Unknown,
            pointer_process: None,
            pointer_role: crate::WindowRole::Unknown,
            active_bindings: Vec::new(),
            requires_process: Vec::new(),
            requires_role: Vec::new(),
            requires_pointer_process: Vec::new(),
            requires_pointer_role: Vec::new(),
            dedicated_leader: None,
            repeat_owner: BTreeMap::new(),
            tap_restore: None,
            layer_owner: None,
            tap_allowed: Vec::new(),
            retained: None,
            last_keyboard_down: None,
            retained_starters: Vec::new(),
            mouse_modifier_gate: None,
            mouse_modifier_context: None,
            uses_mouse_modifier_snapshot: false,
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

    /// 指针身份来自外层快照；与前台身份分别保存，变化仅撤销失去条件的输入。
    pub fn set_pointer_context(
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
        if self.pointer_process == normalized && self.pointer_role == role {
            return Ok(Decision::default());
        }
        self.pointer_process = normalized;
        self.pointer_role = role;
        self.refresh_active_bindings();
        Ok(self.reconcile_conditions())
    }

    pub fn pointer_process(&self) -> Option<&str> {
        self.pointer_process.as_deref()
    }

    pub fn pointer_window_role(&self) -> crate::WindowRole {
        self.pointer_role
    }

    pub fn needs_pointer_context(&self) -> bool {
        self.bindings
            .iter()
            .any(|binding| !binding.pointer_scope.is_global())
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
        self.uses_mouse_modifier_snapshot = self
            .bindings
            .iter()
            .any(|binding| binding.keyboard.matching.is_mouse_sampled());
        let related = |binding: &TriggerSpec, other: &TriggerSpec| {
            if binding.keyboard.matching.is_captured() {
                other.keyboard.matching.is_captured()
                    && binding.keyboard.prefix() == other.keyboard.prefix()
            } else {
                other.held_sets_overlap(binding)
                    && same_activation_family(other.activation, binding.activation)
            }
        };
        self.requires_pointer_process =
            self.bindings
                .iter()
                .map(|binding| {
                    binding.pointer_scope.is_global()
                        && self.bindings.iter().any(|other| {
                            other.pointer_scope.priority() > 0 && related(binding, other)
                        })
                })
                .collect();
        self.requires_pointer_role = self
            .bindings
            .iter()
            .map(|binding| {
                self.bindings
                    .iter()
                    .any(|other| other.pointer_scope.needs_role() && related(binding, other))
            })
            .collect();
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
                            other.held_sets_overlap(binding)
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
                                other.held_sets_overlap(binding)
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
                        || !self.requires_role[index])
                    && binding
                        .pointer_scope
                        .matches_context(self.pointer_process.as_deref(), self.pointer_role)
                    && (self.pointer_process.is_some() || !self.requires_pointer_process[index])
                    && (self.pointer_role != crate::WindowRole::Unknown
                        || !self.requires_pointer_role[index]))
                    .then_some(index)
            })
            .collect();
        self.active_bindings.sort_by_key(|&index| {
            let binding = &self.bindings[index];
            (
                std::cmp::Reverse(binding.scope_priority()),
                binding.keyboard.matching.priority(),
            )
        });
    }

    fn candidates(&self) -> impl Iterator<Item = &TriggerSpec> {
        self.active_bindings
            .iter()
            .copied()
            .filter(move |&index| self.mouse_prefix_allows_binding(index))
            .map(|index| &self.bindings[index])
    }

    fn mouse_prefix_allows_binding(&self, index: usize) -> bool {
        let binding = &self.bindings[index];
        // 与establish_mouse_prefix_session的拥有域一致，不重解释观察/保留层。
        if binding.keyboard.retained_prefix.is_some() || binding.keyboard.matching.is_mouse_rule() {
            return true;
        }
        binding.held.iter().all(|&button| {
            let Button::Mouse(mouse) = button else {
                return true;
            };
            let owners = &self.mouse_prefix_sessions[mouse_slot(mouse)];
            !owners.is_active()
                || !matches!(
                    self.pressed.get(&button),
                    Some(PressState::Pending | PressState::Consumed)
                )
                || owners.contains(index)
        })
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
        self.mouse_modifier_context = None;
        if let Some(gate) = &mut self.mouse_modifier_gate {
            let _ = gate.cancel();
        }
        self.clear_mouse_prefix_sessions();
        self.retained_adorn_order.clear();
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
        self.gesture = None;
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
        self.process_at_inner(event, now_ms, None)
    }

    pub fn needs_mouse_modifier_snapshot(&self) -> bool {
        self.uses_mouse_modifier_snapshot
    }

    /// 调用方明确分配事件流及样本年龄；重新配置时须使用新的stream_id。
    pub fn configure_mouse_modifier_snapshot(
        &mut self,
        stream_id: u64,
        max_age_ms: u32,
    ) -> Result<Decision, crate::MouseSnapshotError> {
        let gate = crate::MouseModifierGate::new(stream_id, max_age_ms)?;
        if self
            .mouse_modifier_gate
            .as_ref()
            .is_some_and(|old| stream_id <= old.stream_id())
        {
            return Err(crate::MouseSnapshotError::StaleStream);
        }
        let decision = self.cancel();
        self.mouse_modifier_gate = Some(gate);
        Ok(decision)
    }

    pub fn mouse_modifier_snapshot_epoch(&self) -> Option<u64> {
        self.mouse_modifier_gate
            .as_ref()
            .map(crate::MouseModifierGate::epoch)
    }

    pub fn process_with_mouse_snapshot_at(
        &mut self,
        event: InputEvent,
        now_ms: u64,
        context: crate::MouseModifierInput,
    ) -> Decision {
        self.process_at_inner(event, now_ms, Some(context))
    }

    fn process_at_inner(
        &mut self,
        event: InputEvent,
        now_ms: u64,
        context: Option<crate::MouseModifierInput>,
    ) -> Decision {
        self.mouse_modifier_context = None;
        if event.injected {
            return Decision::default();
        }
        if let Some(input) = context
            && let Some(gate) = &mut self.mouse_modifier_gate
        {
            self.mouse_modifier_context = gate
                .observe_sample(
                    crate::MouseContextEvent {
                        stream_id: input.stream_id,
                        serial: input.serial,
                        epoch: input.epoch,
                        observed_at_ms: now_ms,
                        kind: event.kind,
                        injected: event.injected,
                    },
                    input.snapshot,
                )
                .ok();
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
        if interrupts_press
            && matches!(event.kind, InputKind::Down(_))
            && self.has_observed_wheel_tail()
        {
            // 先归还上一条观察规则的不足一格输入，再让新按压选择动作或捕获层。
            decision.replay.extend(self.flush_pending());
        }
        if interrupts_press
            && (self.hold.is_some() || self.drag.is_some() || self.gesture_pending())
        {
            decision.replay.extend(self.flush_pending());
        }
        if interrupts_press {
            decision
                .drags
                .extend(self.finish_drag(DragPhase::Cancelled));
            self.cancel_gesture();
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
        self.mouse_modifier_context = None;
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
            if self.hold.is_none() && !self.gesture_pending() {
                decision.replay.extend(self.flush_pending());
            }
        }
        if self
            .gesture
            .as_ref()
            .is_some_and(|gesture| !gesture.started && self.now_ms >= gesture.deadline_ms)
        {
            self.gesture = None;
            // 与拖拽窗口相同：仍在等待的长按继续计时，否则交还原按压。
            if self.hold.is_none() && self.drag.is_none() {
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
            self.gesture
                .as_ref()
                .filter(|gesture| !gesture.started)
                .map(|gesture| gesture.deadline_ms),
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

    /// 手势会话的按键及是否已认领按压；未进入手势或已结束时为 None。
    pub fn gesture_session(&self) -> Option<(Button, bool)> {
        self.gesture
            .as_ref()
            .map(|gesture| (gesture.button, gesture.started))
    }

    fn gesture_pending(&self) -> bool {
        self.gesture
            .as_ref()
            .is_some_and(|gesture| !gesture.started)
    }

    /// 已认领的手势被其他输入打断时放弃识别，消费原按压的释放。
    fn cancel_gesture(&mut self) {
        let Some(gesture) = self.gesture.take_if(|gesture| gesture.started) else {
            return;
        };
        if let Some(state) = self.pressed.get_mut(&gesture.button)
            && *state == PressState::Consumed
        {
            *state = PressState::Draining;
        }
    }

    /// 松开手势键时只在按下时确定的候选中选择，绘制途中指针窗口变化不增删候选。
    /// 先比较含斜向的路径，再比较只含上下左右的路径；同类按候选排序取第一条。
    fn recognize_gesture(&self, gesture: &GestureSession) -> Option<u64> {
        let reading = gesture.tracker.as_ref()?.reading();
        self.bindings
            .iter()
            .enumerate()
            .filter(|&(index, _)| gesture.candidates.contains(index))
            .filter_map(|(index, binding)| match binding.activation {
                Activation::Gesture { path, .. } if reading.matches(&path) => Some((
                    (
                        !path.has_diagonal(),
                        std::cmp::Reverse(binding.scope_priority()),
                        binding.keyboard.matching.priority(),
                        index,
                    ),
                    binding.id,
                )),
                _ => None,
            })
            .min_by_key(|&(key, _)| key)
            .map(|(_, id)| id)
    }

    /// 已建立的鼠标前缀及pending/active拖拽会话，供输入归一化器同步读取。
    /// 普通按住、透传左键前缀、键盘层、按下/长按末键及Draining不建立此会话。
    /// 当前拖拽只接受鼠标终键，因此holding_move_key保持false；
    /// 带键盘前缀的鼠标拖拽仍属于captured_mouse_buttons。
    pub fn wheel_session_snapshot(&self) -> WheelSessionSnapshot {
        let pending = self.drag.map(|drag| drag.button);
        let active = self.active_drag.map(|drag| drag.button);
        WheelSessionSnapshot {
            captured_mouse_buttons: self
                .mouse_prefix_sessions
                .iter()
                .any(|owners| owners.is_active())
                || matches!(pending, Some(Button::Mouse(_)))
                || matches!(active, Some(Button::Mouse(_)))
                || self.gesture.is_some(),
            holding_move_key: false,
        }
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
            if self.drag.is_none() && self.gesture.is_none() {
                decision.replay.extend(self.flush_pending());
            }
        }
        // 鼠标键作为普通前缀等待时，移动说明用户在拖拽（如拖动标题栏、框选）：
        // 放弃组合并立即回放按下，让应用收到真实的按下与后续移动，而不是松开时的一次点击。
        if self.hold.is_none()
            && self.drag.is_none()
            && self.active_drag.is_none()
            && self.gesture.is_none()
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
        if let Some(gesture) = &mut self.gesture {
            let tracker = gesture
                .tracker
                .get_or_insert_with(|| GestureTracker::new(position));
            tracker.push(position);
            if !gesture.started && tracker.started() {
                let mut gesture = self.gesture.take().expect("手势会话存在");
                self.commit_pending();
                gesture.started = true;
                self.gesture = Some(gesture);
            }
        }
        decision
    }

    fn down(&mut self, button: Button) -> Decision {
        if let Some(state) = self.pressed.get(&button).copied() {
            // 普通多键前缀尚未命中时，重复表示继续输入；不能只补回首次Down而丢掉重复。
            // 单前缀（例如原Caps单按）、专用层、鼠标、长按/拖动和已命中终止键均沿旧路径。
            if state == PressState::Pending
                && self.dedicated_leader.is_none()
                && self.retained.is_none()
                && self.hold.is_none()
                && self.drag.is_none()
                && matches!(button, Button::Key(_))
                && !crate::model::is_modifier(button)
                && self.pressed.len() >= 2
                && self.pressed.iter().all(|(&held, &state)| {
                    matches!(held, Button::Key(_))
                        && !self.terminal_presses.contains(&held)
                        && matches!(state, PressState::Pending | PressState::Consumed)
                })
                && self.pending.contains(&InputKind::Down(button))
                && self
                    .pending
                    .iter()
                    .all(|event| matches!(event, InputKind::Down(Button::Key(_))))
            {
                return Decision {
                    replay: self.flush_pending(),
                    ..Decision::default()
                };
            }
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
            .map(|binding| {
                (
                    binding.scope_priority(),
                    binding.keyboard.matching.priority(),
                )
            });
        let press = self
            .candidates()
            .find(|binding| {
                binding.activation == Activation::Press(button)
                    && Some((
                        binding.scope_priority(),
                        binding.keyboard.matching.priority(),
                    )) == priority
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
                    && Some((
                        binding.scope_priority(),
                        binding.keyboard.matching.priority(),
                    )) == priority
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
                    && Some((
                        binding.scope_priority(),
                        binding.keyboard.matching.priority(),
                    )) == priority
                    && self.binding_held_matches(binding)
                    && self.conditions_match(binding) =>
                {
                    Some((binding.id, min_distance_px, max_start_ms))
                }
                _ => None,
            });
        let mut gesture_candidates = MousePrefixOwners::default();
        if drag.is_none() && matches!(button, Button::Mouse(_)) {
            // 各应用范围的手势同时参与：松开时同一路径按候选顺序取最具体的范围。
            for &index in &self.active_bindings {
                let binding = &self.bindings[index];
                if matches!(binding.activation, Activation::Gesture { button: terminal, .. } if terminal == button)
                    && self.mouse_prefix_allows_binding(index)
                    && self.binding_held_matches(binding)
                    && self.conditions_match(binding)
                {
                    gesture_candidates.insert(index);
                }
            }
        }
        let gesture = gesture_candidates.is_active();
        if hold.is_some() || drag.is_some() || gesture {
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
            self.gesture = gesture.then(|| GestureSession {
                button,
                candidates: gesture_candidates,
                tracker: self.pointer_position.map(GestureTracker::new),
                deadline_ms: self
                    .now_ms
                    .saturating_add(u64::from(crate::GESTURE_START_MS)),
                started: false,
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
                        && !binding.keyboard.matching.is_mouse_rule()
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
                if let Button::Mouse(mouse) = button {
                    self.establish_mouse_prefix_session(mouse);
                }
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
        if let Button::Mouse(mouse) = button {
            self.mouse_prefix_sessions[mouse_slot(mouse)] = MousePrefixOwners::default();
        }
        self.repeat_owner.remove(&button);
        self.terminal_presses.remove(&button);
        self.captured_presses.remove(&button);
        let leaving_layer = self.dedicated_leader == Some(button);
        let restore_tap = leaving_layer && self.tap_restore.is_some_and(|tap| tap.button == button);
        if leaving_layer {
            self.clear_mouse_prefix_sessions();
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
        let recognized = match self.gesture.take() {
            Some(gesture) if gesture.button == button && gesture.started => {
                self.recognize_gesture(&gesture)
            }
            Some(gesture) if gesture.button == button => None,
            Some(gesture) => {
                // 其他按键释放：未认领的会话按原回退路径处理，已认领的会话放弃识别。
                self.gesture = Some(gesture);
                if self.pressed.contains_key(&button) {
                    self.cancel_gesture();
                }
                None
            }
            None => None,
        };
        let Some(state) = self.pressed.remove(&button) else {
            return Decision::default();
        };
        let released_pending_drag = self.drag.is_some() || self.gesture_pending();
        if leaving_layer {
            for state in self.pressed.values_mut() {
                if matches!(*state, PressState::Consumed | PressState::PendingSuppressed) {
                    *state = PressState::Draining;
                }
            }
        }
        let observed_wheel = self.has_observed_wheel_tail();
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
            PressState::Passed | PressState::Observed => Decision {
                replay: if observed_wheel || released_pending_drag {
                    self.flush_pending()
                } else {
                    Vec::new()
                },
                ..Decision::default()
            },
        };
        if restore_tap {
            decision
                .replay
                .extend([InputKind::Down(button), InputKind::Up(button)]);
        }
        decision.drags.extend(drag);
        decision.triggered.extend(recognized);
        decision
    }

    fn scroll(&mut self, axis: Axis, delta: i32) -> Decision {
        if delta == 0 || !self.enabled {
            return Decision::default();
        }
        let direction = direction(axis, delta);
        let Some((id, policy)) = self
            .selected_wheel(direction)
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
        if let crate::KeyboardMatching::MouseSampled { policy, .. } = binding.keyboard.matching {
            return self.dedicated_leader.is_none()
                && self.layer_owner.is_none()
                && self.retained.is_none()
                && self.terminal_presses.is_empty()
                && self.captured_presses.is_empty()
                && !self
                    .mouse_prefix_sessions
                    .iter()
                    .any(|owners| owners.is_active())
                && self.pressed.iter().all(|(&button, &state)| {
                    state == PressState::Passed
                        && match button {
                            Button::Key(key) => policy.accepts_passed_key(key),
                            Button::Mouse(_) => policy.allow_extra_passed_mouse,
                        }
                })
                && self
                    .mouse_modifier_gate
                    .as_ref()
                    .zip(self.mouse_modifier_context.as_ref())
                    .is_some_and(|(gate, context)| gate.matches(context, policy).unwrap_or(false));
        }
        if binding.keyboard.matching.is_mouse_unmodified() {
            return self.dedicated_leader.is_none()
                && self.layer_owner.is_none()
                && self.retained.is_none()
                && self.terminal_presses.is_empty()
                && self.captured_presses.is_empty()
                && !self
                    .mouse_prefix_sessions
                    .iter()
                    .any(|owners| owners.is_active())
                && self.pressed.iter().all(|(&button, &state)| {
                    state == PressState::Passed && !crate::model::blocks_unmodified_mouse(button)
                });
        }
        if binding.keyboard.matching.is_mouse_captured_leader() {
            let Some(&leader) = binding.held.first() else {
                return false;
            };
            return self.dedicated_leader == Some(leader)
                && matches!(
                    self.pressed.get(&leader),
                    Some(PressState::Consumed | PressState::Observed)
                )
                && self
                    .layer_owner
                    .zip(self.dedicated_index(leader))
                    .is_some_and(|(owner, current)| {
                        self.bindings[owner]
                            .keyboard
                            .same_layer(&self.bindings[current].keyboard)
                    })
                && self.pressed.iter().all(|(&button, &state)| {
                    !matches!(button, Button::Mouse(_)) || state == PressState::Passed
                });
        }
        if binding.keyboard.matching.is_mouse_exact() {
            return binding.held.iter().all(|&required| {
                self.pressed
                    .keys()
                    .any(|&physical| required == physical || binding.accepts_held_button(physical))
            }) && self.pressed.iter().all(|(&button, &state)| {
                state == PressState::Passed
                    && !self.terminal_presses.contains(&button)
                    && binding.accepts_held_button(button)
            });
        }
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
                && (!binding.conditions.is_empty()
                    || !binding.pointer_scope.is_global()
                    || binding.keyboard.matching.is_mouse_rule()
                    || binding.keyboard.prefix().is_some())
        })
    }

    fn has_observed_wheel_tail(&self) -> bool {
        self.wheel_owner.iter().enumerate().any(|(index, owner)| {
            self.wheel[index] != 0
                && owner.is_some_and(|id| {
                    self.bindings.iter().any(|binding| {
                        binding.id == id && binding.keyboard.matching.is_mouse_rule()
                    })
                })
        })
    }

    fn binding_conditions_match(&self, id: u64) -> bool {
        self.candidates()
            .find(|binding| binding.id == id)
            .is_some_and(|binding| self.conditions_match(binding))
    }

    fn selected_wheel(&self, direction: WheelDirection) -> Option<&TriggerSpec> {
        self.candidates().find(|binding| {
            binding.activation == Activation::Wheel(direction)
                && self.binding_held_matches(binding)
                && self.conditions_match(binding)
        })
    }

    fn observed_wheel_owner_changed(&self, id: u64) -> bool {
        let Some(binding) = self.candidates().find(|binding| binding.id == id) else {
            return false;
        };
        let Activation::Wheel(direction) = binding.activation else {
            return false;
        };
        binding.keyboard.matching.is_mouse_rule()
            && self.selected_wheel(direction).map(|candidate| candidate.id) != Some(id)
    }

    fn reconcile_conditions(&mut self) -> Decision {
        self.reconcile_mouse_prefix_sessions();
        let lost_captures = self
            .captured_presses
            .iter()
            .filter_map(|(&key, &index)| {
                (!self.active_bindings.contains(&index)
                    || !self.conditions_match(&self.bindings[index]))
                .then_some(key)
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
            self.wheel[index] != 0
                && owner.is_some_and(|id| {
                    !self.binding_conditions_match(id) || self.observed_wheel_owner_changed(id)
                })
        });
        let lost_prefix = self.hold.is_none()
            && self.drag.is_none()
            && self.gesture.is_none()
            && self
                .pressed
                .values()
                .any(|state| *state == PressState::Pending)
            && !self.candidates().any(|binding| {
                !binding.keyboard.matching.is_mouse_rule()
                    && self.conditions_match(binding)
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
        self.gesture = None;
        for state in self.pressed.values_mut() {
            if matches!(*state, PressState::Pending | PressState::PendingSuppressed) {
                *state = PressState::Consumed;
            }
        }
    }

    fn flush_pending(&mut self) -> Vec<InputKind> {
        for (&button, &state) in &self.pressed {
            if state == PressState::Pending
                && let Button::Mouse(mouse) = button
            {
                self.mouse_prefix_sessions[mouse_slot(mouse)] = MousePrefixOwners::default();
            }
        }
        self.prefix_origin = None;
        self.hold = None;
        self.drag = None;
        self.gesture = None;
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
            self.clear_mouse_prefix_sessions();
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

    fn clear_mouse_prefix_sessions(&mut self) {
        self.mouse_prefix_sessions = [MousePrefixOwners::default(); MOUSE_BUTTON_COUNT];
    }

    fn establish_mouse_prefix_session(&mut self, mouse: crate::MouseButton) {
        let button = Button::Mouse(mouse);
        let mut owners = MousePrefixOwners::default();
        for &index in &self.active_bindings {
            let binding = &self.bindings[index];
            if binding.keyboard.retained_prefix.is_none()
                && !binding.keyboard.matching.is_mouse_rule()
                && binding.held.contains(&button)
                && self.conditions_match(binding)
                && self
                    .pressed
                    .keys()
                    .all(|pressed| binding.held.contains(pressed))
            {
                owners.insert(index);
            }
        }
        self.mouse_prefix_sessions[mouse_slot(mouse)] = owners;
    }

    fn reconcile_mouse_prefix_sessions(&mut self) {
        if !self
            .mouse_prefix_sessions
            .iter()
            .any(|owners| owners.is_active())
        {
            return;
        }
        let mut available = MousePrefixOwners::default();
        for &index in &self.active_bindings {
            if self.conditions_match(&self.bindings[index]) {
                available.insert(index);
            }
        }
        for mouse in [
            crate::MouseButton::Left,
            crate::MouseButton::Right,
            crate::MouseButton::Middle,
            crate::MouseButton::X1,
            crate::MouseButton::X2,
        ] {
            let owners = &mut self.mouse_prefix_sessions[mouse_slot(mouse)];
            let was_active = owners.is_active();
            for (word, valid) in owners.bindings.iter_mut().zip(&available.bindings) {
                *word &= *valid;
            }
            // 最后一个原会话所有者失效后，只收尾这次已消费的按压。
            // Pending 仍由原回退路径处理；范围恢复不能重新借用旧 Consumed。
            if was_active
                && !owners.is_active()
                && let Some(state) = self.pressed.get_mut(&Button::Mouse(mouse))
                && *state == PressState::Consumed
            {
                *state = PressState::Draining;
            }
        }
    }
}

fn mouse_slot(button: crate::MouseButton) -> usize {
    match button {
        crate::MouseButton::Left => 0,
        crate::MouseButton::Right => 1,
        crate::MouseButton::Middle => 2,
        crate::MouseButton::X1 => 3,
        crate::MouseButton::X2 => 4,
    }
}

fn same_activation_family(left: Activation, right: Activation) -> bool {
    let button = |activation| match activation {
        Activation::Press(button)
        | Activation::Hold { button, .. }
        | Activation::Drag { button, .. }
        | Activation::Gesture { button, .. } => Some(button),
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

#[cfg(test)]
mod mouse_unmodified_tests {
    use super::*;
    use crate::{KeyboardMatching, KeyboardPolicy, MouseButton};

    const Q: Button = Button::Key(0x51);
    const WIN: Button = Button::Key(0x5b);
    const RIGHT: Button = Button::Mouse(MouseButton::Right);

    fn rule(id: u64, priority: u16, activation: Activation) -> TriggerSpec {
        TriggerSpec {
            id,
            held: vec![],
            activation,
            conditions: vec![],
            keyboard: KeyboardPolicy {
                matching: KeyboardMatching::MouseUnmodified {
                    priority,
                    pointer_scope_priority: true,
                },
                ..Default::default()
            },
            pointer_scope: Default::default(),
            process_scope: Default::default(),
            wheel: if matches!(activation, Activation::Wheel(_)) {
                WheelPolicy::Packet
            } else {
                WheelPolicy::Notch
            },
        }
    }

    fn wheel(id: u64, priority: u16) -> TriggerSpec {
        rule(id, priority, Activation::Wheel(WheelDirection::Up))
    }

    fn input(engine: &mut Engine, kind: InputKind) -> Decision {
        engine.process(InputEvent {
            kind,
            injected: false,
        })
    }

    #[test]
    fn unmodified_matching_requires_passed_state_for_every_keyboard_and_mouse_press() {
        let binding = wheel(1, 0);
        let mut engine = Engine::new(vec![binding.clone()]).unwrap();
        for button in [Q, WIN, Button::Mouse(MouseButton::X1)] {
            for state in [
                PressState::Pending,
                PressState::PendingSuppressed,
                PressState::Consumed,
                PressState::Passed,
                PressState::Draining,
                PressState::Observed,
            ] {
                engine.pressed.clear();
                engine.pressed.insert(button, state);
                assert_eq!(
                    engine.binding_held_matches(&binding),
                    state == PressState::Passed
                );
            }
        }
        for key in [0x10, 0x11, 0x12, 0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5] {
            engine.pressed.clear();
            engine.pressed.insert(Button::Key(key), PressState::Passed);
            assert!(!engine.binding_held_matches(&binding));
        }
    }

    #[test]
    fn unmodified_matching_rejects_terminal_and_session_ledgers_even_with_passed_keys() {
        let binding = wheel(1, 0);
        let mut engine = Engine::new(vec![binding.clone()]).unwrap();
        engine.pressed.insert(Q, PressState::Passed);
        assert!(engine.binding_held_matches(&binding));
        engine.terminal_presses.insert(Q);
        assert!(!engine.binding_held_matches(&binding));
        engine.terminal_presses.clear();
        engine.captured_presses.insert(Q, 0);
        assert!(!engine.binding_held_matches(&binding));
        engine.captured_presses.clear();
        engine.dedicated_leader = Some(Q);
        assert!(!engine.binding_held_matches(&binding));
        engine.dedicated_leader = None;
        engine.layer_owner = Some(0);
        assert!(!engine.binding_held_matches(&binding));
        engine.layer_owner = None;
        engine.mouse_prefix_sessions[0].insert(0);
        assert!(!engine.binding_held_matches(&binding));
        engine.clear_mouse_prefix_sessions();
        assert!(engine.binding_held_matches(&binding));
    }

    #[test]
    fn unmodified_mouse_trigger_never_claims_extra_passed_presses_or_their_repeats_and_releases() {
        let mut engine = Engine::new(vec![rule(1, 0, Activation::Press(RIGHT))]).unwrap();
        for button in [Q, WIN, Button::Mouse(MouseButton::X1)] {
            assert_eq!(
                input(&mut engine, InputKind::Down(button)),
                Decision::default()
            );
        }
        let triggered = input(&mut engine, InputKind::Down(RIGHT));
        assert!(triggered.consume);
        assert_eq!(triggered.triggered, [1]);
        assert!(
            input(&mut engine, InputKind::Down(RIGHT))
                .triggered
                .is_empty()
        );
        for button in [Q, WIN, Button::Mouse(MouseButton::X1)] {
            assert_eq!(engine.pressed[&button], PressState::Passed);
            assert_eq!(
                input(&mut engine, InputKind::Down(button)),
                Decision::default()
            );
        }
        engine.cancel();
        for button in [Q, WIN, Button::Mouse(MouseButton::X1)] {
            assert_eq!(
                input(&mut engine, InputKind::Up(button)),
                Decision::default()
            );
        }
        assert!(input(&mut engine, InputKind::Up(RIGHT)).consume);
    }

    #[test]
    fn unmodified_rule_does_not_borrow_exact_prefix_ownership_at_equal_rank() {
        let mut exact = wheel(2, 0);
        exact.keyboard.matching = KeyboardMatching::Exact;
        exact.held = vec![Q];
        let mut engine = Engine::new(vec![wheel(1, 0), exact]).unwrap();
        assert!(input(&mut engine, InputKind::Down(Q)).consume);
        let wheel_event = InputKind::Wheel {
            axis: Axis::Vertical,
            delta: 120,
        };
        assert_eq!(input(&mut engine, wheel_event).triggered, [2]);
        assert!(input(&mut engine, InputKind::Up(Q)).consume);
        assert_eq!(input(&mut engine, wheel_event).triggered, [1]);
    }

    #[test]
    fn strict_mouse_win_and_unmodified_rules_use_the_same_explicit_rank() {
        for (priority, expected) in [(1, 1), (3, 2)] {
            let mut strict = wheel(2, 2);
            strict.keyboard.matching = KeyboardMatching::MouseExact {
                priority: 2,
                pointer_scope_priority: true,
            };
            strict.held = vec![WIN];
            let mut engine = Engine::new(vec![wheel(1, priority), strict]).unwrap();
            assert_eq!(
                input(&mut engine, InputKind::Down(WIN)),
                Decision::default()
            );
            assert_eq!(
                input(
                    &mut engine,
                    InputKind::Wheel {
                        axis: Axis::Vertical,
                        delta: 120
                    }
                )
                .triggered,
                [expected]
            );
            assert_eq!(input(&mut engine, InputKind::Up(WIN)), Decision::default());
        }
    }

    #[test]
    fn captured_layer_keeps_mouse_priority_even_when_unmodified_is_sorted_first() {
        for priority in [0, 5] {
            let mut owner = rule(3, 0, Activation::Press(Button::Key(0x4a)));
            owner.held = vec![Q];
            owner.keyboard = KeyboardPolicy {
                matching: KeyboardMatching::Captured { priority: 0 },
                dedicated_prefix: Some(Q),
                ..Default::default()
            };
            let mut captured = wheel(2, priority);
            captured.held = vec![Q];
            captured.keyboard.matching = KeyboardMatching::MouseCapturedLeader {
                priority,
                pointer_scope_priority: true,
            };
            let mut engine = Engine::new(vec![wheel(1, 0), captured, owner]).unwrap();
            assert!(input(&mut engine, InputKind::Down(Q)).consume);
            let wheel_event = InputKind::Wheel {
                axis: Axis::Vertical,
                delta: 120,
            };
            assert_eq!(input(&mut engine, wheel_event).triggered, [2]);
            assert!(input(&mut engine, InputKind::Up(Q)).consume);
            assert_eq!(input(&mut engine, wheel_event).triggered, [1]);
        }
    }

    #[test]
    fn retained_capture_and_unmodified_same_rank_keep_their_separate_ownership_domains() {
        let mut captured = wheel(2, 0);
        captured.held = vec![Q];
        captured.keyboard = KeyboardPolicy {
            matching: KeyboardMatching::Captured { priority: 0 },
            retained_prefix: Some(Box::new(crate::RetainedPrefix {
                key: Q,
                capture_scope: Default::default(),
                quiet_ms: 0,
                fast_ms: 0,
                block_with_primary_mouse: false,
            })),
            restore_on_tap: true,
            ..Default::default()
        };
        let mut engine = Engine::new(vec![wheel(1, 0), captured]).unwrap();
        assert!(input(&mut engine, InputKind::Down(Q)).consume);
        let wheel_event = InputKind::Wheel {
            axis: Axis::Vertical,
            delta: 120,
        };
        assert_eq!(input(&mut engine, wheel_event).triggered, [2]);
        assert!(input(&mut engine, InputKind::Up(Q)).consume);
        assert_eq!(input(&mut engine, wheel_event).triggered, [1]);
    }

    #[test]
    fn explicit_rank_applies_to_press_vs_timed_candidates_for_the_same_mouse_button() {
        let mut hold = rule(
            2,
            0,
            Activation::Hold {
                button: RIGHT,
                duration_ms: 50,
            },
        );
        hold.keyboard = KeyboardPolicy::default();
        hold.wheel = WheelPolicy::Notch;
        let mut engine = Engine::new(vec![rule(1, 1, Activation::Press(RIGHT)), hold]).unwrap();
        let pending = input(&mut engine, InputKind::Down(RIGHT));
        assert!(pending.consume);
        assert!(pending.triggered.is_empty());
        assert_eq!(engine.advance_time(50).triggered, [2]);
        assert!(input(&mut engine, InputKind::Up(RIGHT)).consume);
    }

    #[test]
    fn strict_mouse_matching_remains_closed_to_extra_ordinary_and_win_keys() {
        let mut strict = wheel(1, 0);
        strict.keyboard.matching = KeyboardMatching::MouseExact {
            priority: 0,
            pointer_scope_priority: true,
        };
        for button in [Q, WIN, Button::Mouse(MouseButton::X1)] {
            let mut engine = Engine::new(vec![strict.clone()]).unwrap();
            assert_eq!(
                input(&mut engine, InputKind::Down(button)),
                Decision::default()
            );
            assert_eq!(
                input(
                    &mut engine,
                    InputKind::Wheel {
                        axis: Axis::Vertical,
                        delta: 120
                    }
                ),
                Decision::default()
            );
        }
    }
}

#[cfg(test)]
mod mouse_sampled_tests {
    use super::*;
    use crate::{
        KeyboardMatching, KeyboardPolicy, ModifierRequirement, ModifierSides, MouseModifierInput,
        MouseModifierPolicy, MouseModifierSnapshot,
    };

    fn policy(required: ModifierRequirement) -> MouseModifierPolicy {
        MouseModifierPolicy {
            required,
            allow_other_modifiers: required == ModifierRequirement::NoControlShiftAlt,
            allow_extra_passed_keys: true,
            allow_extra_passed_mouse: false,
        }
    }
    fn rule(id: u64, required: ModifierRequirement, priority: u16) -> TriggerSpec {
        TriggerSpec {
            id,
            held: vec![],
            activation: Activation::Wheel(WheelDirection::Up),
            conditions: vec![],
            keyboard: KeyboardPolicy {
                matching: KeyboardMatching::MouseSampled {
                    policy: policy(required),
                    priority,
                    pointer_scope_priority: true,
                },
                ..Default::default()
            },
            pointer_scope: Default::default(),
            process_scope: Default::default(),
            wheel: WheelPolicy::Packet,
        }
    }
    fn wheel() -> InputEvent {
        InputEvent {
            kind: InputKind::Wheel {
                axis: Axis::Vertical,
                delta: 120,
            },
            injected: false,
        }
    }
    fn context(engine: &Engine, serial: u64, time: u64, control: bool) -> MouseModifierInput {
        let epoch = engine.mouse_modifier_snapshot_epoch().unwrap();
        MouseModifierInput {
            stream_id: 7,
            serial,
            epoch,
            snapshot: Some(MouseModifierSnapshot {
                stream_id: 7,
                event_serial: serial,
                epoch,
                sampled_at_ms: time,
                modifiers: ModifierSides {
                    left_control: control,
                    ..Default::default()
                },
            }),
        }
    }

    #[test]
    fn legacy_entry_never_borrows_previous_sample_and_explicit_context_is_per_event() {
        let mut engine = Engine::new(vec![rule(1, ModifierRequirement::Control, 0)]).unwrap();
        assert!(engine.needs_mouse_modifier_snapshot());
        assert!(engine.process_at(wheel(), 0).triggered.is_empty());
        engine.configure_mouse_modifier_snapshot(7, 0).unwrap();
        let input = context(&engine, 1, 10, true);
        assert_eq!(
            engine
                .process_with_mouse_snapshot_at(wheel(), 10, input)
                .triggered,
            [1]
        );
        assert!(engine.process_at(wheel(), 11).triggered.is_empty());
        assert!(engine.mouse_modifier_context.is_none());
        assert!(
            engine
                .process_with_mouse_snapshot_at(wheel(), 11, input)
                .triggered
                .is_empty()
        );
    }

    #[test]
    fn missing_expired_wrong_stream_and_injected_samples_fail_closed_without_clock_leakage() {
        let mut engine = Engine::new(vec![rule(1, ModifierRequirement::Control, 0)]).unwrap();
        engine.configure_mouse_modifier_snapshot(7, 5).unwrap();
        let mut missing = context(&engine, 1, 10, true);
        missing.snapshot = None;
        assert!(
            engine
                .process_with_mouse_snapshot_at(wheel(), 10, missing)
                .triggered
                .is_empty()
        );
        let stale = context(&engine, 2, 4, true);
        assert!(
            engine
                .process_with_mouse_snapshot_at(wheel(), 10, stale)
                .triggered
                .is_empty()
        );
        let mut wrong = context(&engine, 3, 10, true);
        wrong.stream_id = 8;
        assert!(
            engine
                .process_with_mouse_snapshot_at(wheel(), 10, wrong)
                .triggered
                .is_empty()
        );
        let valid = context(&engine, 4, 10, true);
        assert_eq!(
            engine.process_with_mouse_snapshot_at(
                InputEvent {
                    injected: true,
                    ..wheel()
                },
                100,
                valid
            ),
            Decision::default()
        );
        assert_eq!(
            engine
                .process_with_mouse_snapshot_at(wheel(), 10, valid)
                .triggered,
            [1]
        );
        let reversed = context(&engine, 5, 9, true);
        assert!(
            engine
                .process_with_mouse_snapshot_at(wheel(), 9, reversed)
                .triggered
                .is_empty()
        );
    }

    #[test]
    fn same_snapshot_keeps_original_order_without_unmodified_stealing_sampled_control() {
        let mut engine = Engine::new(vec![
            rule(1, ModifierRequirement::NoControlShiftAlt, 0),
            rule(2, ModifierRequirement::Control, 2),
        ])
        .unwrap();
        engine.configure_mouse_modifier_snapshot(7, 0).unwrap();
        let ctrl = context(&engine, 1, 10, true);
        assert_eq!(
            engine
                .process_with_mouse_snapshot_at(wheel(), 10, ctrl)
                .triggered,
            [2]
        );
        let none = context(&engine, 2, 11, false);
        assert_eq!(
            engine
                .process_with_mouse_snapshot_at(wheel(), 11, none)
                .triggered,
            [1]
        );
        assert_eq!(
            engine.process_at(
                InputEvent {
                    kind: InputKind::Down(Button::Key(65)),
                    injected: false
                },
                12
            ),
            Decision::default()
        );
        let extra = context(&engine, 3, 13, true);
        assert_eq!(
            engine
                .process_with_mouse_snapshot_at(wheel(), 13, extra)
                .triggered,
            [2]
        );
        assert_eq!(
            engine.process_at(
                InputEvent {
                    kind: InputKind::Up(Button::Key(65)),
                    injected: false
                },
                14
            ),
            Decision::default()
        );
    }

    #[test]
    fn owned_prefix_and_capture_sessions_reject_sampled_mouse_without_borrowing_keys() {
        let key = Button::Key(65);
        let mut owner = rule(2, ModifierRequirement::Control, 0);
        owner.held = vec![key];
        owner.activation = Activation::Press(Button::Key(66));
        owner.wheel = WheelPolicy::Notch;
        owner.keyboard = KeyboardPolicy {
            matching: KeyboardMatching::Captured { priority: 0 },
            dedicated_prefix: Some(key),
            ..Default::default()
        };
        let mut engine =
            Engine::new(vec![rule(1, ModifierRequirement::Control, 0), owner]).unwrap();
        engine.configure_mouse_modifier_snapshot(7, 0).unwrap();
        assert!(
            engine
                .process_at(
                    InputEvent {
                        kind: InputKind::Down(key),
                        injected: false
                    },
                    10
                )
                .consume
        );
        let blocked = context(&engine, 1, 11, true);
        assert!(
            engine
                .process_with_mouse_snapshot_at(wheel(), 11, blocked)
                .triggered
                .is_empty()
        );
        assert!(
            engine
                .process_at(
                    InputEvent {
                        kind: InputKind::Up(key),
                        injected: false
                    },
                    12
                )
                .consume
        );
        let next = context(&engine, 2, 13, true);
        assert_eq!(
            engine
                .process_with_mouse_snapshot_at(wheel(), 13, next)
                .triggered,
            [1]
        );
    }

    #[test]
    fn cancellation_revokes_sample_epoch_but_consumed_mouse_release_still_finishes() {
        let button = Button::Mouse(crate::MouseButton::X1);
        let mut press = rule(1, ModifierRequirement::Control, 0);
        press.activation = Activation::Press(button);
        press.wheel = WheelPolicy::Notch;
        let mut engine = Engine::new(vec![press]).unwrap();
        engine.configure_mouse_modifier_snapshot(7, 0).unwrap();
        let input = context(&engine, 1, 10, true);
        let down = InputEvent {
            kind: InputKind::Down(button),
            injected: false,
        };
        assert_eq!(
            engine
                .process_with_mouse_snapshot_at(down, 10, input)
                .triggered,
            [1]
        );
        engine.cancel();
        assert!(
            engine
                .process_at(
                    InputEvent {
                        kind: InputKind::Up(button),
                        injected: false
                    },
                    11
                )
                .consume
        );
        assert!(
            engine
                .process_with_mouse_snapshot_at(down, 12, MouseModifierInput { serial: 2, ..input })
                .triggered
                .is_empty()
        );
        assert!(
            !engine
                .process_at(
                    InputEvent {
                        kind: InputKind::Up(button),
                        injected: false
                    },
                    13
                )
                .consume
        );
        let fresh = context(&engine, 3, 14, true);
        assert_eq!(
            engine
                .process_with_mouse_snapshot_at(down, 14, fresh)
                .triggered,
            [1]
        );
    }

    #[test]
    fn notch_remainder_uses_fresh_snapshot_and_returns_tail_when_context_is_missing() {
        let mut binding = rule(1, ModifierRequirement::Control, 0);
        binding.wheel = WheelPolicy::Notch;
        let mut engine = Engine::new(vec![binding]).unwrap();
        engine.configure_mouse_modifier_snapshot(7, 0).unwrap();
        let half = InputEvent {
            kind: InputKind::Wheel {
                axis: Axis::Vertical,
                delta: 60,
            },
            injected: false,
        };
        let first = context(&engine, 1, 10, true);
        assert!(
            engine
                .process_with_mouse_snapshot_at(half, 10, first)
                .triggered
                .is_empty()
        );
        assert_eq!(engine.advance_time(11).replay, [half.kind]);
        let next = context(&engine, 2, 12, true);
        assert!(
            engine
                .process_with_mouse_snapshot_at(half, 12, next)
                .triggered
                .is_empty()
        );
        let last = context(&engine, 3, 13, true);
        assert_eq!(
            engine
                .process_with_mouse_snapshot_at(half, 13, last)
                .triggered,
            [1]
        );
    }

    #[test]
    fn reconfiguration_requires_increasing_stream_identity_and_rejects_old_payloads() {
        let mut engine = Engine::new(vec![rule(1, ModifierRequirement::Control, 0)]).unwrap();
        engine.configure_mouse_modifier_snapshot(7, 0).unwrap();
        let old = context(&engine, 1, 10, true);
        assert_eq!(
            engine.configure_mouse_modifier_snapshot(7, 0).unwrap_err(),
            crate::MouseSnapshotError::StaleStream
        );
        assert_eq!(
            engine.configure_mouse_modifier_snapshot(6, 0).unwrap_err(),
            crate::MouseSnapshotError::StaleStream
        );
        assert_eq!(engine.mouse_modifier_snapshot_epoch(), Some(1));
        engine.configure_mouse_modifier_snapshot(8, 0).unwrap();
        assert!(
            engine
                .process_with_mouse_snapshot_at(wheel(), 10, old)
                .triggered
                .is_empty()
        );
        let mut next = old;
        next.stream_id = 8;
        next.snapshot.as_mut().unwrap().stream_id = 8;
        assert_eq!(
            engine
                .process_with_mouse_snapshot_at(wheel(), 10, next)
                .triggered,
            [1]
        );
    }
}
