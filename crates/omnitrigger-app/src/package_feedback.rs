//! 动作包调试反馈：只在内存中保留有限预览，按提交时的调用身份和参数匹配。
use omnitrigger_actions::{ActionCall, ActionEvent, ActionOutcome};
use std::collections::{BTreeMap, VecDeque};

const HISTORY_LIMIT: usize = 4;
const PREVIEW_BYTES: usize = 8192;

struct Submitted {
    binding: u64,
    call: ActionCall,
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
            self.active.insert(invocation, Submitted { binding, call });
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
}
