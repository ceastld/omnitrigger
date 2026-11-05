//! 亮度/音量共用的取消、绝对期限与写入阶段；不创建线程或执行器。
use omnitrigger_actions::CancellationToken;
use omnitrigger_windows::{SystemAdjustmentError, SystemAdjustmentErrorCode};
use std::{cell::Cell, time::Instant};

pub(super) struct AdjustmentControl<'a> {
    cancellation: &'a CancellationToken,
    deadline: Option<Instant>,
    now: &'a dyn Fn() -> Instant,
    written: Cell<bool>,
    stopped: Cell<Option<&'static str>>,
}
impl<'a> AdjustmentControl<'a> {
    pub fn new(
        cancellation: &'a CancellationToken,
        deadline: Option<Instant>,
        now: &'a dyn Fn() -> Instant,
    ) -> Self {
        Self {
            cancellation,
            deadline,
            now,
            written: Cell::new(false),
            stopped: Cell::new(None),
        }
    }
    pub fn check(&self, write: bool) -> Result<(), SystemAdjustmentError> {
        let reason = if self.cancellation.is_cancelled() {
            Some("cancelled")
        } else if self.deadline.is_some_and(|limit| (self.now)() >= limit) {
            Some("timeout")
        } else {
            None
        };
        if let Some(reason) = reason {
            self.stopped.set(Some(reason));
            return Err(SystemAdjustmentError {
                code: SystemAdjustmentErrorCode::ConfirmationUnknown,
                message: "系统调节已停止。".into(),
            });
        }
        if write {
            self.written.set(true);
        }
        Ok(())
    }
    pub fn effect(&self) -> &'static str {
        if self.written.get() {
            "unknown"
        } else {
            "not_started"
        }
    }
    pub fn reason(&self) -> Option<&'static str> {
        self.stopped.get()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_control_preserves_prewrite_and_postwrite_cancel_deadline_without_waiting() {
        for written in [false, true] {
            let token = CancellationToken::new();
            let start = Instant::now();
            let now = Cell::new(start);
            let clock = || now.get();
            let guard = AdjustmentControl::new(
                &token,
                Some(start + std::time::Duration::from_millis(1)),
                &clock,
            );
            guard.check(written).unwrap();
            now.set(start + std::time::Duration::from_millis(1));
            assert!(guard.check(false).is_err());
            assert_eq!(guard.reason(), Some("timeout"));
            assert_eq!(
                guard.effect(),
                if written { "unknown" } else { "not_started" }
            );
        }
        let token = CancellationToken::new();
        let clock = Instant::now;
        let guard = AdjustmentControl::new(&token, None, &clock);
        token.cancel();
        assert!(guard.check(true).is_err());
        assert_eq!(guard.reason(), Some("cancelled"));
        assert_eq!(guard.effect(), "not_started");
    }
}
