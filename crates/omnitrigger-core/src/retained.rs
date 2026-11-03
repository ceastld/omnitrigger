//! 保留原功能前缀的有界会话；不读取时钟、系统按键状态或执行输入。
use super::{Engine, PRIMARY, PressState};
use crate::{Activation, Button, Decision, InputKind};

#[derive(Clone, Copy)]
pub(super) struct RetainedSession {
    key: Button,
    owner: usize,
    started_ms: u64,
    temporary: Option<Button>,
    used: bool,
    restored_down: bool,
}

impl Engine {
    pub(super) fn retained_event(&mut self, event: InputKind) -> Option<Decision> {
        let Some(session) = self.retained else {
            let InputKind::Down(button @ Button::Key(_)) = event else {
                return None;
            };
            if self.dedicated_leader.is_some() {
                return None;
            }
            let previous = self.last_keyboard_down;
            if self.enabled
                && !self.pressed.contains_key(&button)
                && let Some(owner) = self.retained_starters.iter().copied().find(|&index| {
                    self.bindings[index]
                        .keyboard
                        .retained_prefix
                        .as_ref()
                        .is_some_and(|policy| policy.key == button)
                        && self.conditions_match(&self.bindings[index])
                })
            {
                let policy = self.bindings[owner]
                    .keyboard
                    .retained_prefix
                    .as_ref()
                    .unwrap();
                let blocked = previous.is_some_and(|time| {
                    self.now_ms.saturating_sub(time) < u64::from(policy.quiet_ms)
                }) || (policy.block_with_primary_mouse
                    && self.pressed.contains_key(&PRIMARY));
                let replay = self.flush_pending();
                if blocked {
                    self.last_keyboard_down = Some(self.now_ms);
                    self.pressed.insert(button, PressState::Passed);
                    return Some(Decision {
                        replay,
                        ..Decision::default()
                    });
                }
                self.dedicated_leader = Some(button);
                self.layer_owner = Some(owner);
                self.pressed.insert(button, PressState::Consumed);
                self.retained = Some(RetainedSession {
                    key: button,
                    owner,
                    started_ms: self.now_ms,
                    temporary: None,
                    used: false,
                    restored_down: false,
                });
                return Some(Decision {
                    consume: true,
                    replay,
                    ..Decision::default()
                });
            }
            self.last_keyboard_down = Some(self.now_ms);
            return None;
        };
        match event {
            InputKind::Down(button @ Button::Key(_)) => Some(self.retained_down(session, button)),
            InputKind::Up(button @ Button::Key(_)) => self.retained_up(session, button),
            _ => {
                let mut replay = Vec::new();
                if event == InputKind::Down(PRIMARY) && !session.restored_down {
                    replay.push(InputKind::Down(session.key));
                    self.pressed.insert(session.key, PressState::Observed);
                    self.retained.as_mut().unwrap().restored_down = true;
                }
                let mut decision = match event {
                    InputKind::Down(button) => self.down(button),
                    InputKind::Up(button) => self.up(button),
                    InputKind::Wheel { axis, delta } => self.scroll(axis, delta),
                };
                if decision.triggered.iter().any(|id| {
                    self.bindings
                        .iter()
                        .any(|binding| binding.id == *id && binding.held.contains(&session.key))
                }) && let Some(active) = self.retained.as_mut()
                {
                    active.used = true;
                }
                replay.extend(decision.replay);
                decision.replay = replay;
                Some(decision)
            }
        }
    }

    fn retained_action(&self, button: Button) -> Option<(u64, bool)> {
        self.candidates()
            .find(|binding| {
                binding.activation == Activation::Press(button)
                    && self.binding_held_matches(binding)
                    && self.conditions_match(binding)
            })
            .map(|binding| (binding.id, binding.keyboard.repeat))
    }

    fn retained_capture(&mut self, button: Button, owner: usize) {
        // 此前已经交付给应用的按下仍须交付释放，不能因为重复事件被捕获而丢掉释放。
        self.pressed.entry(button).or_insert(PressState::Consumed);
        self.captured_presses.insert(button, owner);
    }

    fn retained_hit(&mut self, button: Button, id: u64, repeat: bool) -> Decision {
        self.commit_pending();
        self.pressed.entry(button).or_insert(PressState::Consumed);
        self.terminal_presses.insert(button);
        if repeat {
            self.repeat_owner.insert(button, id);
        }
        Decision {
            consume: true,
            triggered: vec![id],
            ..Decision::default()
        }
    }

    fn retained_down(&mut self, session: RetainedSession, button: Button) -> Decision {
        if button == session.key {
            return Decision {
                consume: true,
                ..Decision::default()
            };
        }
        let repeating = self.pressed.contains_key(&button);
        self.retained.as_mut().unwrap().used = true;
        if let Some(temporary) = session.temporary {
            let owner = self.captured_extension(temporary).unwrap_or(session.owner);
            self.retained_capture(temporary, owner);
            if let Some((id, repeat)) = self.retained_action(button) {
                self.retained.as_mut().unwrap().temporary = None;
                if !repeating || repeat {
                    return self.retained_hit(button, id, repeat);
                }
            } else {
                // 暂存键后的未命中第三键同样计入会话捕获集合。
                self.retained_capture(button, session.owner);
            }
            return Decision {
                consume: true,
                ..Decision::default()
            };
        }
        let policy = self.bindings[session.owner]
            .keyboard
            .retained_prefix
            .as_ref()
            .unwrap();
        if !session.restored_down
            && self.now_ms.saturating_sub(session.started_ms) < u64::from(policy.fast_ms)
        {
            if self.captured_extension(button).is_some() {
                self.pressed.entry(button).or_insert(PressState::Consumed);
                self.retained.as_mut().unwrap().temporary = Some(button);
                return Decision {
                    consume: true,
                    ..Decision::default()
                };
            }
            return self.retained_fallback(session, button);
        }
        if let Some((id, repeat)) = self.retained_action(button) {
            if !repeating || repeat {
                return self.retained_hit(button, id, repeat);
            }
            return Decision {
                consume: true,
                ..Decision::default()
            };
        }
        if let Some(owner) = self.captured_extension(button) {
            self.retained_capture(button, owner);
            return Decision {
                consume: true,
                ..Decision::default()
            };
        }
        self.retained_fallback(session, button)
    }

    fn finish_retained(&mut self) {
        self.retained = None;
        self.dedicated_leader = None;
        self.layer_owner = None;
        self.captured_presses.clear();
        self.repeat_owner.clear();
        self.terminal_presses.clear();
        for state in self.pressed.values_mut() {
            if matches!(state, PressState::Consumed | PressState::PendingSuppressed) {
                *state = PressState::Draining;
            }
        }
        self.wheel = [0; 4];
        self.wheel_owner = [None; 4];
    }

    fn retained_fallback(&mut self, session: RetainedSession, button: Button) -> Decision {
        let mut replay = self.flush_pending();
        replay.push(InputKind::Down(session.key));
        self.finish_retained();
        self.pressed.insert(session.key, PressState::Observed);
        self.pressed.entry(button).or_insert(PressState::Passed);
        Decision {
            replay,
            ..Decision::default()
        }
    }

    fn retained_up(&mut self, session: RetainedSession, button: Button) -> Option<Decision> {
        if session.temporary == Some(button) {
            let action = self.retained_action(button);
            if action.is_some() {
                self.commit_pending();
            }
            self.retained.as_mut().unwrap().temporary = None;
            self.retained.as_mut().unwrap().used = true;
            let consume = !matches!(
                self.pressed.remove(&button),
                Some(PressState::Passed | PressState::Observed)
            );
            self.repeat_owner.remove(&button);
            self.terminal_presses.remove(&button);
            // 暂存键被第三键提升为捕获键后，先释放暂存键仍保留该会话的捕获资格。
            // 这项显式时序语义只属于保留原功能模式，直到引导键释放或取消时清除。
            return Some(Decision {
                consume,
                triggered: action.map(|action| action.0).into_iter().collect(),
                ..Decision::default()
            });
        }
        if button != session.key {
            return None;
        }
        let mut replay = self.flush_pending();
        let restores_tap = session.temporary.is_some()
            || (!session.used
                && self.tap_allowed[session.owner]
                && self.bindings[session.owner]
                    .keyboard
                    .tap_timeout_ms
                    .is_none_or(|ms| {
                        self.now_ms.saturating_sub(session.started_ms) <= u64::from(ms)
                    }));
        if let Some(temporary) = session.temporary {
            replay.extend([
                InputKind::Down(button),
                InputKind::Up(button),
                InputKind::Down(temporary),
                InputKind::Up(temporary),
            ]);
        } else if !session.used {
            self.last_keyboard_down = Some(self.now_ms);
            let policy = &self.bindings[session.owner].keyboard;
            if self.tap_allowed[session.owner]
                && policy.tap_timeout_ms.is_none_or(|ms| {
                    self.now_ms.saturating_sub(session.started_ms) <= u64::from(ms)
                })
            {
                replay.extend([InputKind::Down(button), InputKind::Up(button)]);
            }
        }
        let consume = !session.restored_down || restores_tap;
        self.finish_retained();
        self.pressed.remove(&button);
        Some(Decision {
            consume,
            replay,
            ..Decision::default()
        })
    }
}
