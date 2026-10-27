//! 从标准化事件录制统一触发描述，不读取平台状态。

use std::collections::BTreeSet;

use crate::model::direction;
use crate::{Activation, Button, InputEvent, InputKind, TriggerSpec};

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
                        conditions: vec![],
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
                    conditions: vec![],
                    id: 0,
                    held: self.pressed.iter().copied().collect(),
                    activation: Activation::Wheel(direction(axis, delta)),
                })
            }
            InputKind::Wheel { .. } => None,
        }
    }
}
