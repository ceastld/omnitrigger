//! 动作包调试反馈：只在内存中保留有限预览，按提交时的调用身份和参数匹配。
use omnitrigger_actions::{ActionCall, ActionEvent, ActionOutcome};
use std::collections::{BTreeMap, VecDeque};

const HISTORY_LIMIT: usize = 4;
const PREVIEW_BYTES: usize = 8192;

struct Submitted {
    binding: u64,
    call: ActionCall,
    action_hint: Option<String>,
}

pub(super) struct ResultPreview {
    pub invocation: u64,
    pub label: &'static str,
    pub text: String,
    pub truncated: bool,
}
struct Completed {
    submitted: Submitted,
    preview: ResultPreview,
}

#[derive(Default)]
pub(super) struct PackageFeedback {
    active: BTreeMap<u64, Submitted>,
    completed: VecDeque<Completed>,
    expanded: Option<u64>,
}

pub(super) enum Feedback<'a> {
    Running(u64),
    Completed(&'a ResultPreview),
}

impl PackageFeedback {
    pub fn set_action_hint(
        &mut self,
        update: &omnitrigger_actions::action_hint::ScopedActionHint,
    ) -> Result<(), omnitrigger_actions::ActionError> {
        use omnitrigger_actions::action_hint::{fault, validate_text};
        validate_text(&update.text)?;
        let submitted = self
            .active
            .get_mut(&update.scope.invocation_id)
            .filter(|submitted| {
                submitted.binding == update.scope.binding_id
                    && update.scope.matches_call(&submitted.call)
            })
            .ok_or_else(|| fault("ui_hint_scope_expired", "not_started"))?;
        submitted.action_hint = Some(update.text.clone());
        Ok(())
    }

    pub fn action_hint(&self, invocation: u64) -> Option<&str> {
        self.active
            .get(&invocation)
            .and_then(|submitted| submitted.action_hint.as_deref())
            .or_else(|| {
                self.completed
                    .iter()
                    .find(|run| run.preview.invocation == invocation)
                    .and_then(|run| run.submitted.action_hint.as_deref())
            })
    }

    pub fn action_hint_for_call(&self, call: &ActionCall) -> Option<&str> {
        let same_action = |submitted: &Submitted| {
            submitted.call.package_id == call.package_id
                && submitted.call.action_id == call.action_id
                && submitted.call.action_version == call.action_version
        };
        self.active
            .iter()
            .rev()
            .find_map(|(_, submitted)| {
                same_action(submitted)
                    .then_some(submitted.action_hint.as_deref())
                    .flatten()
            })
            .or_else(|| {
                self.completed.iter().find_map(|run| {
                    same_action(&run.submitted)
                        .then_some(run.submitted.action_hint.as_deref())
                        .flatten()
                })
            })
    }

    pub fn toggle_details(&mut self, invocation: u64) {
        if self
            .completed
            .iter()
            .any(|run| run.preview.invocation == invocation)
        {
            self.expanded = if self.expanded == Some(invocation) {
                None
            } else {
                Some(invocation)
            };
        }
    }

    pub fn details_visible(&self, invocation: u64) -> bool {
        self.expanded == Some(invocation)
    }
    pub fn accepted(&mut self, binding: u64, invocation: u64, call: ActionCall) {
        // 调用服务最多保留 CAPACITY 项；不为未接纳的请求创建记录。
        if self.active.len() < omnitrigger_actions::service::CAPACITY {
            self.active.insert(
                invocation,
                Submitted {
                    binding,
                    call,
                    action_hint: None,
                },
            );
        }
    }

    pub fn apply(&mut self, event: &ActionEvent) {
        if !event.outcome.is_terminal()
            || self
                .active
                .get(&event.invocation_id)
                .is_none_or(|run| run.binding != event.id)
        {
            return;
        }
        let submitted = self
            .active
            .remove(&event.invocation_id)
            .expect("checked above");
        let (label, text) = match &event.outcome {
            ActionOutcome::Succeeded { output } => ("已完成", serde_json::to_string_pretty(output)),
            ActionOutcome::Failed { error } => ("失败", serde_json::to_string_pretty(error)),
            ActionOutcome::Cancelled => ("已停止", Ok("已完成的操作不会撤销。".into())),
            ActionOutcome::Started => unreachable!(),
        };
        let mut text = text.unwrap_or_else(|error| format!("结果无法显示：{error}"));
        let truncated = text.len() > PREVIEW_BYTES;
        if truncated {
            let mut end = PREVIEW_BYTES;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }
        self.completed.push_front(Completed {
            submitted,
            preview: ResultPreview {
                invocation: event.invocation_id,
                label,
                text,
                truncated,
            },
        });
        self.completed.truncate(HISTORY_LIMIT);
    }

    pub fn for_call(&self, binding: u64, call: &ActionCall) -> Option<Feedback<'_>> {
        if let Some((invocation, _)) = self
            .active
            .iter()
            .find(|(_, run)| run.binding == binding && run.call == *call)
        {
            return Some(Feedback::Running(*invocation));
        }
        self.completed
            .iter()
            .find(|run| run.submitted.binding == binding && run.submitted.call == *call)
            .map(|run| Feedback::Completed(&run.preview))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnitrigger_actions::ActionError;
    use serde_json::json;

    fn call(text: &str) -> ActionCall {
        ActionCall {
            package_id: "test".into(),
            action_id: "run".into(),
            action_version: 1,
            input: json!({"text":text}),
        }
    }
    fn finished(invocation: u64, outcome: ActionOutcome) -> ActionEvent {
        ActionEvent {
            id: 1,
            invocation_id: invocation,
            outcome,
        }
    }
    fn preview<'a>(feedback: &'a PackageFeedback, call: &ActionCall) -> &'a ResultPreview {
        let Some(Feedback::Completed(preview)) = feedback.for_call(1, call) else {
            panic!("missing completed preview");
        };
        preview
    }

    #[test]
    fn feedback_matches_original_call_and_keeps_running_and_terminal_states_distinct() {
        let mut feedback = PackageFeedback::default();
        let first = call("first");
        feedback.accepted(1, 1, first.clone());
        feedback.apply(&finished(1, ActionOutcome::Started));
        assert!(matches!(
            feedback.for_call(1, &first),
            Some(Feedback::Running(1))
        ));
        assert!(feedback.for_call(1, &call("edited")).is_none());
        assert!(feedback.for_call(2, &first).is_none());
        feedback.apply(&finished(
            1,
            ActionOutcome::Succeeded {
                output: json!({"value":false,"empty":null}),
            },
        ));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&preview(&feedback, &first).text).unwrap(),
            json!({"value":false,"empty":null})
        );
        feedback.toggle_details(1);
        assert!(feedback.details_visible(1));
        feedback.accepted(1, 2, first.clone());
        assert!(matches!(
            feedback.for_call(1, &first),
            Some(Feedback::Running(2))
        ));
        feedback.apply(&finished(2, ActionOutcome::Cancelled));
        assert_eq!(preview(&feedback, &first).label, "已停止");
        assert!(!feedback.details_visible(2));
        assert!(feedback.active.is_empty());
    }

    #[test]
    fn feedback_ignores_unrelated_or_duplicate_terminal_events() {
        let mut feedback = PackageFeedback::default();
        let original = call("original");
        feedback.accepted(1, 8, original.clone());
        let mut wrong = finished(
            8,
            ActionOutcome::Succeeded {
                output: json!("wrong binding"),
            },
        );
        wrong.id = 2;
        feedback.apply(&wrong);
        feedback.apply(&finished(
            7,
            ActionOutcome::Succeeded {
                output: json!("old invocation"),
            },
        ));
        assert!(matches!(
            feedback.for_call(1, &original),
            Some(Feedback::Running(8))
        ));
        feedback.apply(&finished(
            8,
            ActionOutcome::Succeeded {
                output: json!("correct"),
            },
        ));
        feedback.apply(&finished(
            8,
            ActionOutcome::Succeeded {
                output: json!("duplicate"),
            },
        ));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&preview(&feedback, &original).text).unwrap(),
            json!("correct")
        );
        assert_eq!(feedback.completed.len(), 1);
    }

    #[test]
    fn previews_bound_history_and_unicode_without_changing_structured_errors() {
        let mut feedback = PackageFeedback::default();
        let original = call("example");
        let mut error = ActionError::new("target_changed", "目标已经改变");
        error.effect_state = Some("not_started".into());
        error.method = Some("desktop.sendText".into());
        feedback.accepted(1, 1, original.clone());
        feedback.apply(&finished(1, ActionOutcome::Failed { error }));
        let parsed: serde_json::Value =
            serde_json::from_str(&preview(&feedback, &original).text).unwrap();
        assert_eq!(parsed["code"], "target_changed");
        assert_eq!(parsed["effect_state"], "not_started");
        for invocation in 2..=7 {
            feedback.accepted(1, invocation, original.clone());
            feedback.apply(&finished(
                invocation,
                ActionOutcome::Succeeded {
                    output: json!("中文🙂".repeat(4096)),
                },
            ));
        }
        let latest = preview(&feedback, &original);
        assert!(latest.truncated);
        assert!(latest.text.len() <= PREVIEW_BYTES);
        assert_eq!(latest.invocation, 7);
        assert_eq!(feedback.completed.len(), HISTORY_LIMIT);
        assert!(feedback.active.is_empty());
    }
    fn hint_update(
        binding: u64,
        invocation: u64,
        text: &str,
    ) -> omnitrigger_actions::action_hint::ScopedActionHint {
        omnitrigger_actions::action_hint::ScopedActionHint {
            scope: omnitrigger_actions::action_hint::ActionHintScope {
                binding_id: binding,
                invocation_id: invocation,
                package_id: "test".into(),
                action_id: "run".into(),
                action_version: 1,
            },
            text: text.into(),
        }
    }

    #[test]
    fn action_hint_updates_only_the_exact_active_invocation_and_definition() {
        let mut feedback = PackageFeedback::default();
        let original = call("original");
        feedback.accepted(1, 10, original.clone());
        feedback.accepted(2, 11, call("other"));
        feedback
            .set_action_hint(&hint_update(1, 10, "n10"))
            .unwrap();
        assert_eq!(feedback.action_hint(10), Some("n10"));
        assert_eq!(feedback.action_hint(11), None);
        for mut update in [
            hint_update(2, 10, "pre"),
            hint_update(1, 999, "pre"),
            hint_update(1, 10, "pre"),
        ] {
            if update.scope.binding_id == 1 && update.scope.invocation_id == 10 {
                update.scope.action_version = 2;
            }
            assert_eq!(
                feedback.set_action_hint(&update).unwrap_err().code,
                "ui_hint_scope_expired"
            );
        }
        let mut other_definition = hint_update(1, 10, "pre");
        other_definition.scope.action_id = "other".into();
        assert!(feedback.set_action_hint(&other_definition).is_err());
        assert!(feedback
            .set_action_hint(&hint_update(1, 10, "bad\ntext"))
            .is_err());
        assert_eq!(feedback.action_hint(10), Some("n10"));
    }

    #[test]
    fn action_hint_survives_failure_as_version_information_and_expires_with_existing_history() {
        let mut feedback = PackageFeedback::default();
        let original = call("original");
        feedback.accepted(1, 1, original.clone());
        feedback.set_action_hint(&hint_update(1, 1, "pre")).unwrap();
        feedback.apply(&finished(
            1,
            ActionOutcome::Failed {
                error: ActionError::new("example_failed", "failed"),
            },
        ));
        assert_eq!(feedback.action_hint(1), Some("pre"));
        assert_eq!(preview(&feedback, &original).label, "失败");
        assert_eq!(
            feedback
                .set_action_hint(&hint_update(1, 1, "n10"))
                .unwrap_err()
                .code,
            "ui_hint_scope_expired"
        );
        assert_eq!(feedback.action_hint_for_call(&original), Some("pre"));
        for invocation in 2..=6 {
            feedback.accepted(1, invocation, original.clone());
            feedback.apply(&finished(
                invocation,
                ActionOutcome::Succeeded {
                    output: json!(null),
                },
            ));
        }
        assert_eq!(feedback.completed.len(), HISTORY_LIMIT);
        assert_eq!(feedback.action_hint(1), None);
        assert_eq!(feedback.action_hint_for_call(&original), None);
    }

    #[test]
    fn action_hint_library_lookup_uses_definition_but_never_changes_other_versions() {
        let mut feedback = PackageFeedback::default();
        let original = call("first input");
        feedback.accepted(1, 1, original.clone());
        feedback.set_action_hint(&hint_update(1, 1, "n10")).unwrap();
        assert_eq!(
            feedback.action_hint_for_call(&call("different input")),
            Some("n10")
        );
        let mut other = original.clone();
        other.action_version = 2;
        assert_eq!(feedback.action_hint_for_call(&other), None);
        other = original;
        other.package_id = "different".into();
        assert_eq!(feedback.action_hint_for_call(&other), None);
    }
}
