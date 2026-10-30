//! 亮度 HUD 的请求目标估计；只消费请求与回执，不读取或监控硬件。
//!
//! 展示值可以先于硬件确认，不能将它描述为已经生效的实际亮度。
use std::collections::{HashMap, VecDeque};

#[derive(Default)]
pub(super) struct HudValues {
    monitors: HashMap<String, MonitorValues>,
}

#[derive(Default)]
struct MonitorValues {
    latest_request: Option<u64>,
    last_result: Option<u64>,
    actual: Option<u8>,
    display: Option<u8>,
    pending: VecDeque<(u64, i32)>,
}

impl HudValues {
    /// 每个已成功提交的请求只累加一次；首次实际值未知时保持未知。
    pub(super) fn begin(&mut self, token: u64, monitor: &str, delta: i32) -> Option<u8> {
        let values = self.monitors.entry(monitor.to_owned()).or_default();
        if values.latest_request.is_some_and(|latest| token <= latest) {
            return values.display;
        }

        // 较旧回执可以补上未知基线；尚未完成的逐次增量仍需计入。
        // 必须逐次 clamp，例如 95 + 10 - 10 的请求目标应为 90。
        let previous = values.display.or_else(|| {
            values.actual.map(|actual| {
                values
                    .pending
                    .iter()
                    .fold(actual, |value, (_, delta)| adjusted(value, *delta))
            })
        });
        values.latest_request = Some(token);
        values.pending.push_back((token, delta));
        values.display = previous.map(|value| adjusted(value, delta));
        values.display
    }

    /// 合并批次的 token 涵盖此前请求；旧回执补基线，但不覆盖较新展示目标。
    pub(super) fn complete(&mut self, token: u64, monitor: &str, actual: u8) -> Option<u8> {
        let values = self.monitors.get_mut(monitor)?;
        if !values.accepts_result(token) {
            return values.display;
        }

        values.resolve_through(token);
        values.actual = Some(actual.min(100));
        if values.latest_request == Some(token) {
            values.display = values.actual;
        }
        values.display
    }

    /// 写入后读回也可能失败，因此相关屏幕的估计和实际基线都需失效。
    /// 已有较新成功回执时，旧失败不再影响该屏，也不影响其他屏幕。
    pub(super) fn fail(&mut self, token: u64) {
        for values in self.monitors.values_mut() {
            if values.accepts_result(token) {
                values.resolve_through(token);
                values.actual = None;
                values.display = None;
                return;
            }
        }
    }

    #[cfg(test)]
    pub(super) fn display(&self, monitor: &str) -> Option<u8> {
        self.monitors.get(monitor).and_then(|values| values.display)
    }
}

impl MonitorValues {
    fn accepts_result(&self, token: u64) -> bool {
        self.last_result.is_none_or(|resolved| token > resolved)
            && self.pending.iter().any(|(pending, _)| *pending == token)
    }

    fn resolve_through(&mut self, token: u64) {
        self.last_result = Some(token);
        while self
            .pending
            .front()
            .is_some_and(|(pending, _)| *pending <= token)
        {
            self.pending.pop_front();
        }
    }
}

fn adjusted(value: u8, delta: i32) -> u8 {
    (i64::from(value) + i64::from(delta)).clamp(0, 100) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(value: u8) -> HudValues {
        let mut values = HudValues::default();
        assert_eq!(values.begin(1, "A", 5), None);
        assert_eq!(values.complete(1, "A", value), Some(value));
        values
    }

    #[test]
    fn unknown_first_request_waits_for_actual_then_targets_respond_immediately() {
        let mut values = known(30);
        assert_eq!(values.begin(2, "A", 5), Some(35));
        assert_eq!(values.begin(3, "A", 5), Some(40));
        assert_eq!(values.display("missing"), None);
    }

    #[test]
    fn duplicate_or_older_begin_does_not_accumulate_twice() {
        let mut values = known(30);
        assert_eq!(values.begin(3, "A", 5), Some(35));
        assert_eq!(values.begin(3, "A", 5), Some(35));
        assert_eq!(values.begin(2, "A", 20), Some(35));
        assert_eq!(values.complete(3, "A", 35), Some(35));
        assert_eq!(values.begin(3, "A", 5), Some(35));
    }

    #[test]
    fn merged_completion_covers_all_requests_without_reapplying_deltas() {
        let mut values = known(30);
        assert_eq!(values.begin(2, "A", 5), Some(35));
        assert_eq!(values.begin(3, "A", 5), Some(40));
        assert_eq!(values.complete(3, "A", 39), Some(39));
        assert_eq!(values.begin(4, "A", 5), Some(44));
        assert_eq!(values.complete(2, "A", 35), Some(44));
        assert_eq!(values.complete(4, "A", 44), Some(44));
    }

    #[test]
    fn old_completion_preserves_new_target_and_latest_actual_corrects_it() {
        let mut values = known(30);
        values.begin(2, "A", 5);
        values.begin(3, "A", 5);
        assert_eq!(values.complete(2, "A", 34), Some(40));
        assert_eq!(values.begin(4, "A", 5), Some(45));
        assert_eq!(values.complete(3, "A", 39), Some(45));
        assert_eq!(values.complete(4, "A", 43), Some(43));
    }

    #[test]
    fn unknown_baseline_keeps_unfinished_deltas_after_old_completion() {
        let mut values = HudValues::default();
        assert_eq!(values.begin(1, "A", 5), None);
        assert_eq!(values.begin(2, "A", 5), None);
        assert_eq!(values.complete(1, "A", 35), None);
        assert_eq!(values.begin(3, "A", 5), Some(45));
        assert_eq!(values.complete(3, "A", 45), Some(45));
    }

    #[test]
    fn monitor_values_and_old_cross_screen_results_are_independent() {
        let mut values = known(30);
        values.begin(2, "B", -5);
        values.complete(2, "B", 75);
        assert_eq!(values.begin(3, "A", 5), Some(35));
        assert_eq!(values.begin(4, "B", -5), Some(70));
        assert_eq!(values.complete(3, "A", 35), Some(35));
        assert_eq!(values.display("B"), Some(70));
        values.fail(4);
        assert_eq!(values.display("B"), None);
        assert_eq!(values.display("A"), Some(35));
    }

    #[test]
    fn clamping_applies_per_request_and_accepts_extreme_deltas() {
        let mut values = known(95);
        assert_eq!(values.begin(2, "A", 10), Some(100));
        assert_eq!(values.begin(3, "A", -10), Some(90));
        assert_eq!(values.begin(4, "A", i32::MAX), Some(100));
        assert_eq!(values.begin(5, "A", i32::MIN), Some(0));
        assert_eq!(values.complete(5, "A", 255), Some(100));
    }

    #[test]
    fn unknown_baseline_replays_clamped_requests_in_order() {
        let mut values = HudValues::default();
        values.begin(1, "A", 5);
        values.begin(2, "A", 10);
        values.begin(3, "A", -10);
        values.complete(1, "A", 95);
        assert_eq!(values.begin(4, "A", 5), Some(95));
    }

    #[test]
    fn failure_invalidates_until_success_without_reusing_failed_estimates() {
        let mut values = known(30);
        values.begin(2, "A", 5);
        values.begin(3, "A", 5);
        values.fail(2);
        assert_eq!(values.display("A"), None);
        assert_eq!(values.begin(4, "A", 5), None);
        assert_eq!(values.complete(2, "A", 35), None);
        assert_eq!(values.complete(3, "A", 41), None);
        assert_eq!(values.begin(5, "A", 5), Some(51));
        assert_eq!(values.complete(5, "A", 50), Some(50));
    }

    #[test]
    fn stale_or_unknown_failure_cannot_invalidate_newer_success() {
        let mut values = known(30);
        values.begin(2, "A", 5);
        values.begin(3, "A", 5);
        values.complete(3, "A", 40);
        values.fail(2);
        values.fail(3);
        values.fail(u64::MAX);
        assert_eq!(values.begin(4, "A", 5), Some(45));
    }

    #[test]
    fn unmatched_completion_cannot_create_or_change_monitor_values() {
        let mut values = known(30);
        assert_eq!(values.complete(20, "missing", 70), None);
        assert_eq!(values.complete(20, "A", 70), Some(30));
        assert_eq!(values.display("A"), Some(30));
    }
}
