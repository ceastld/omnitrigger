//! 独立、有界的鼠标动作调度策略；只输出调度决定，不执行动作、不安装定时器。
//! 调用方在Hook外使用，并显式提供启用状态、时间、取消与原目标上下文标识。

use std::collections::VecDeque;

pub const MAX_DELAYED_DISPATCHES: usize = 64;
pub const MAX_BUILTIN_MOUSE_SWITCHES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusyMousePolicy {
    Drop,
    /// FIFO等待只限制尚未开始的工作，deadline处过期，不重试已开始的动作。
    Delay {
        capacity: usize,
        max_wait_ms: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseDispatchClass {
    Configured,
    Builtin { feature_id: u64 },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DispatchEnablement {
    Enabled,
    Disabled,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuiltinMouseSwitch {
    pub feature_id: u64,
    pub state: DispatchEnablement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseDispatchControls<'a> {
    pub epoch: u64,
    pub configured: DispatchEnablement,
    /// 每项独立提供；缺少某个feature即Unknown，不推断其他开关或个人设置。
    pub builtin: &'a [BuiltinMouseSwitch],
}

impl MouseDispatchControls<'_> {
    fn state(self, class: MouseDispatchClass) -> DispatchEnablement {
        match class {
            MouseDispatchClass::Configured => self.configured,
            MouseDispatchClass::Builtin { feature_id } => self
                .builtin
                .iter()
                .find(|switch| switch.feature_id == feature_id)
                .map_or(DispatchEnablement::Unknown, |switch| switch.state),
        }
    }

    fn validate(self) -> Result<(), MouseDispatchError> {
        if self.builtin.len() > MAX_BUILTIN_MOUSE_SWITCHES
            || self.builtin.iter().enumerate().any(|(index, switch)| {
                switch.feature_id == 0
                    || self.builtin[..index]
                        .iter()
                        .any(|other| other.feature_id == switch.feature_id)
            })
        {
            return Err(MouseDispatchError::InvalidControls);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseDispatchRequest {
    pub serial: u64,
    pub epoch: u64,
    pub binding_id: u64,
    /// 动作版本与原目标身份在外层保存；这里只携带不可变上下文引用，不重抓前台或重绑动作。
    pub context_id: u64,
    pub class: MouseDispatchClass,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActiveMouseDispatch {
    pub token: u64,
    pub request: MouseDispatchRequest,
    pub cancellation_requested: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseDispatchDropReason {
    Busy,
    QueueFull,
    Disabled,
    EnablementUnknown,
    Expired,
    Cancelled,
    ContextInvalidated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DroppedMouseDispatch {
    pub token: u64,
    pub request: MouseDispatchRequest,
    pub reason: MouseDispatchDropReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseDispatchAdmission {
    Started(ActiveMouseDispatch),
    Delayed { token: u64, deadline_ms: u64 },
    Dropped(MouseDispatchDropReason),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MouseDispatchUpdate {
    pub started: Option<ActiveMouseDispatch>,
    pub dropped: Vec<DroppedMouseDispatch>,
    /// 仅请求取消，active继续占用，直到外层明确complete。
    pub cancel_active: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MouseDispatchCancellation {
    pub update: MouseDispatchUpdate,
    /// None表示代次耗尽，已关闭接纳；现有active仍可complete以完成清理。
    pub next_epoch: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseDispatchError {
    InvalidPolicy,
    Closed,
    InvalidRequest,
    StaleRequest,
    EpochMismatch,
    ClockWentBackward,
    DeadlineOverflow,
    TokenExhausted,
    CompletionMismatch,
    InvalidContext,
    InvalidControls,
}

#[derive(Clone, Copy)]
struct WaitingDispatch {
    active: ActiveMouseDispatch,
    deadline_ms: u64,
}

pub struct MouseDispatchQueue {
    policy: BusyMousePolicy,
    epoch: u64,
    next_token: Option<u64>,
    last_serial: u64,
    last_time_ms: Option<u64>,
    active: Option<ActiveMouseDispatch>,
    waiting: VecDeque<WaitingDispatch>,
    closed: bool,
}

impl MouseDispatchQueue {
    pub fn new(policy: BusyMousePolicy) -> Result<Self, MouseDispatchError> {
        let capacity = match policy {
            BusyMousePolicy::Drop => 0,
            BusyMousePolicy::Delay {
                capacity,
                max_wait_ms,
            } if (1..=MAX_DELAYED_DISPATCHES).contains(&capacity)
                && (1..=60_000).contains(&max_wait_ms) =>
            {
                capacity
            }
            _ => return Err(MouseDispatchError::InvalidPolicy),
        };
        Ok(Self {
            policy,
            epoch: 1,
            next_token: Some(1),
            last_serial: 0,
            last_time_ms: None,
            active: None,
            waiting: VecDeque::with_capacity(capacity),
            closed: false,
        })
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn active(&self) -> Option<ActiveMouseDispatch> {
        self.active
    }
    pub fn waiting_len(&self) -> usize {
        self.waiting.len()
    }
    pub fn next_deadline_ms(&self) -> Option<u64> {
        self.waiting.front().map(|waiting| waiting.deadline_ms)
    }

    fn check_frame(
        &self,
        now_ms: u64,
        controls: MouseDispatchControls<'_>,
    ) -> Result<(), MouseDispatchError> {
        if self.closed {
            return Err(MouseDispatchError::Closed);
        }
        if controls.epoch != self.epoch {
            return Err(MouseDispatchError::EpochMismatch);
        }
        controls.validate()?;
        self.check_time(now_ms)
    }

    fn check_time(&self, now_ms: u64) -> Result<(), MouseDispatchError> {
        if self.last_time_ms.is_some_and(|last| now_ms < last) {
            return Err(MouseDispatchError::ClockWentBackward);
        }
        Ok(())
    }

    pub fn offer(
        &mut self,
        request: MouseDispatchRequest,
        now_ms: u64,
        controls: MouseDispatchControls<'_>,
    ) -> Result<MouseDispatchAdmission, MouseDispatchError> {
        self.check_frame(now_ms, controls)?;
        if request.epoch != self.epoch {
            return Err(MouseDispatchError::EpochMismatch);
        }
        if request.serial == 0
            || request.binding_id == 0
            || request.context_id == 0
            || matches!(request.class, MouseDispatchClass::Builtin { feature_id: 0 })
        {
            return Err(MouseDispatchError::InvalidRequest);
        }
        if request.serial <= self.last_serial {
            return Err(MouseDispatchError::StaleRequest);
        }
        // 拒绝的事件也不能在启用状态或忙状态改变后以相同serial自动重试。
        self.last_serial = request.serial;
        self.last_time_ms = Some(now_ms);
        if let Some(reason) = disabled_reason(controls.state(request.class)) {
            return Ok(MouseDispatchAdmission::Dropped(reason));
        }
        let deadline = if self.active.is_some() {
            match self.policy {
                BusyMousePolicy::Drop => {
                    return Ok(MouseDispatchAdmission::Dropped(
                        MouseDispatchDropReason::Busy,
                    ));
                }
                BusyMousePolicy::Delay {
                    capacity,
                    max_wait_ms,
                } => {
                    if self.waiting.len() >= capacity {
                        return Ok(MouseDispatchAdmission::Dropped(
                            MouseDispatchDropReason::QueueFull,
                        ));
                    }
                    Some(
                        now_ms
                            .checked_add(u64::from(max_wait_ms))
                            .ok_or(MouseDispatchError::DeadlineOverflow)?,
                    )
                }
            }
        } else {
            None
        };
        let token = self.next_token.ok_or(MouseDispatchError::TokenExhausted)?;
        self.next_token = token.checked_add(1);
        let active = ActiveMouseDispatch {
            token,
            request,
            cancellation_requested: false,
        };
        if let Some(deadline_ms) = deadline {
            self.waiting.push_back(WaitingDispatch {
                active,
                deadline_ms,
            });
            Ok(MouseDispatchAdmission::Delayed { token, deadline_ms })
        } else {
            self.active = Some(active);
            Ok(MouseDispatchAdmission::Started(active))
        }
    }

    /// 外层在开关变化及队列deadline处调用；Unknown不借用旧开关值。
    pub fn advance(
        &mut self,
        now_ms: u64,
        controls: MouseDispatchControls<'_>,
    ) -> Result<MouseDispatchUpdate, MouseDispatchError> {
        self.check_frame(now_ms, controls)?;
        self.last_time_ms = Some(now_ms);
        Ok(self.reconcile(now_ms, controls, None))
    }

    pub fn invalidate_context(
        &mut self,
        context_id: u64,
        now_ms: u64,
        controls: MouseDispatchControls<'_>,
    ) -> Result<MouseDispatchUpdate, MouseDispatchError> {
        if context_id == 0 {
            return Err(MouseDispatchError::InvalidContext);
        }
        self.check_frame(now_ms, controls)?;
        self.last_time_ms = Some(now_ms);
        Ok(self.reconcile(now_ms, controls, Some(context_id)))
    }

    /// complete必须晚于实际运行和资源清理；错误token不释放任何占用。
    pub fn complete(
        &mut self,
        token: u64,
        now_ms: u64,
        controls: MouseDispatchControls<'_>,
    ) -> Result<MouseDispatchUpdate, MouseDispatchError> {
        if self.active.is_none_or(|active| active.token != token) {
            return Err(MouseDispatchError::CompletionMismatch);
        }
        if self.closed {
            self.check_time(now_ms)?;
        } else {
            self.check_frame(now_ms, controls)?;
        }
        self.last_time_ms = Some(now_ms);
        self.active = None;
        if self.closed {
            return Ok(MouseDispatchUpdate::default());
        }
        Ok(self.reconcile(now_ms, controls, None))
    }

    pub fn cancel(&mut self) -> MouseDispatchCancellation {
        let mut update = MouseDispatchUpdate::default();
        while let Some(waiting) = self.waiting.pop_front() {
            update.dropped.push(DroppedMouseDispatch {
                token: waiting.active.token,
                request: waiting.active.request,
                reason: MouseDispatchDropReason::Cancelled,
            });
        }
        if let Some(active) = &mut self.active {
            if !active.cancellation_requested {
                update.cancel_active = Some(active.token);
            }
            active.cancellation_requested = true;
        }
        let next_epoch = if self.closed {
            None
        } else {
            self.epoch.checked_add(1)
        };
        if let Some(epoch) = next_epoch {
            self.epoch = epoch;
        } else {
            self.closed = true;
        }
        MouseDispatchCancellation { update, next_epoch }
    }

    fn reconcile(
        &mut self,
        now_ms: u64,
        controls: MouseDispatchControls<'_>,
        invalid_context: Option<u64>,
    ) -> MouseDispatchUpdate {
        let mut update = MouseDispatchUpdate::default();
        if let Some(active) = &mut self.active
            && (invalid_context == Some(active.request.context_id)
                || disabled_reason(controls.state(active.request.class)).is_some())
            && !active.cancellation_requested
        {
            active.cancellation_requested = true;
            update.cancel_active = Some(active.token);
        }
        // 每次至多检查64项，保留未被撤销项的FIFO顺序。
        for _ in 0..self.waiting.len() {
            let waiting = self
                .waiting
                .pop_front()
                .expect("队列长度在本轮有界迭代内保持有效");
            let reason = if now_ms >= waiting.deadline_ms {
                Some(MouseDispatchDropReason::Expired)
            } else if invalid_context == Some(waiting.active.request.context_id) {
                Some(MouseDispatchDropReason::ContextInvalidated)
            } else {
                disabled_reason(controls.state(waiting.active.request.class))
            };
            if let Some(reason) = reason {
                update.dropped.push(DroppedMouseDispatch {
                    token: waiting.active.token,
                    request: waiting.active.request,
                    reason,
                });
            } else {
                self.waiting.push_back(waiting);
            }
        }
        if self.active.is_none()
            && let Some(waiting) = self.waiting.pop_front()
        {
            self.active = Some(waiting.active);
            update.started = self.active;
        }
        update
    }
}

fn disabled_reason(state: DispatchEnablement) -> Option<MouseDispatchDropReason> {
    match state {
        DispatchEnablement::Enabled => None,
        DispatchEnablement::Disabled => Some(MouseDispatchDropReason::Disabled),
        DispatchEnablement::Unknown => Some(MouseDispatchDropReason::EnablementUnknown),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn controls(epoch: u64) -> MouseDispatchControls<'static> {
        MouseDispatchControls {
            epoch,
            configured: DispatchEnablement::Enabled,
            builtin: &[
                BuiltinMouseSwitch {
                    feature_id: 1,
                    state: DispatchEnablement::Enabled,
                },
                BuiltinMouseSwitch {
                    feature_id: 2,
                    state: DispatchEnablement::Enabled,
                },
            ],
        }
    }
    fn request(serial: u64, epoch: u64, class: MouseDispatchClass) -> MouseDispatchRequest {
        MouseDispatchRequest {
            serial,
            epoch,
            class,
            binding_id: serial,
            context_id: 100 + serial,
        }
    }
    fn queue() -> MouseDispatchQueue {
        MouseDispatchQueue::new(BusyMousePolicy::Delay {
            capacity: 2,
            max_wait_ms: 10,
        })
        .unwrap()
    }
    fn token(admission: MouseDispatchAdmission) -> u64 {
        match admission {
            MouseDispatchAdmission::Started(active) => active.token,
            MouseDispatchAdmission::Delayed { token, .. } => token,
            _ => panic!("测试期望接纳"),
        }
    }

    #[test]
    fn drop_policy_has_no_queue_and_rejected_serial_cannot_be_retried() {
        let mut q = MouseDispatchQueue::new(BusyMousePolicy::Drop).unwrap();
        let first = token(
            q.offer(
                request(1, 1, MouseDispatchClass::Configured),
                0,
                controls(1),
            )
            .unwrap(),
        );
        assert_eq!(
            q.offer(
                request(2, 1, MouseDispatchClass::Configured),
                1,
                controls(1)
            )
            .unwrap(),
            MouseDispatchAdmission::Dropped(MouseDispatchDropReason::Busy)
        );
        q.complete(first, 2, controls(1)).unwrap();
        assert_eq!(
            q.offer(
                request(2, 1, MouseDispatchClass::Configured),
                3,
                controls(1)
            )
            .unwrap_err(),
            MouseDispatchError::StaleRequest
        );
        assert_eq!(q.waiting_len(), 0);
        assert!(q.active().is_none());
    }

    #[test]
    fn delayed_fifo_is_bounded_and_preserves_original_request_context() {
        let mut q = queue();
        let first = token(
            q.offer(
                request(1, 1, MouseDispatchClass::Configured),
                0,
                controls(1),
            )
            .unwrap(),
        );
        let second = token(
            q.offer(
                request(2, 1, MouseDispatchClass::Configured),
                1,
                controls(1),
            )
            .unwrap(),
        );
        let third = token(
            q.offer(
                request(3, 1, MouseDispatchClass::Builtin { feature_id: 1 }),
                2,
                controls(1),
            )
            .unwrap(),
        );
        assert_eq!(
            q.offer(
                request(4, 1, MouseDispatchClass::Builtin { feature_id: 1 }),
                3,
                controls(1)
            )
            .unwrap(),
            MouseDispatchAdmission::Dropped(MouseDispatchDropReason::QueueFull)
        );
        assert_eq!(q.waiting_len(), 2);
        let next = q.complete(first, 4, controls(1)).unwrap().started.unwrap();
        assert_eq!((next.token, next.request.context_id), (second, 102));
        assert_eq!(
            q.complete(second, 5, controls(1))
                .unwrap()
                .started
                .unwrap()
                .token,
            third
        );
        assert!(q.complete(third, 6, controls(1)).unwrap().started.is_none());
    }

    #[test]
    fn deadline_is_exclusive_and_expired_work_is_never_started_or_retried() {
        let mut q = queue();
        let first = token(
            q.offer(
                request(1, 1, MouseDispatchClass::Configured),
                0,
                controls(1),
            )
            .unwrap(),
        );
        let second = token(
            q.offer(
                request(2, 1, MouseDispatchClass::Configured),
                1,
                controls(1),
            )
            .unwrap(),
        );
        assert_eq!(q.next_deadline_ms(), Some(11));
        assert!(q.advance(10, controls(1)).unwrap().dropped.is_empty());
        let expired = q.advance(11, controls(1)).unwrap();
        assert_eq!(expired.dropped[0].token, second);
        assert_eq!(expired.dropped[0].reason, MouseDispatchDropReason::Expired);
        assert!(
            q.complete(first, 12, controls(1))
                .unwrap()
                .started
                .is_none()
        );
        assert_eq!(q.next_deadline_ms(), None);
    }

    #[test]
    fn builtin_and_configured_switches_are_explicit_and_unknown_never_uses_previous_value() {
        let mut q = queue();
        let unknown = MouseDispatchControls {
            builtin: &[],
            ..controls(1)
        };
        assert_eq!(
            q.offer(
                request(1, 1, MouseDispatchClass::Builtin { feature_id: 1 }),
                0,
                unknown
            )
            .unwrap(),
            MouseDispatchAdmission::Dropped(MouseDispatchDropReason::EnablementUnknown)
        );
        let active = token(
            q.offer(request(2, 1, MouseDispatchClass::Configured), 1, unknown)
                .unwrap(),
        );
        q.offer(
            request(3, 1, MouseDispatchClass::Builtin { feature_id: 1 }),
            2,
            controls(1),
        )
        .unwrap();
        let update = q.advance(3, unknown).unwrap();
        assert_eq!(
            update.dropped[0].reason,
            MouseDispatchDropReason::EnablementUnknown
        );
        assert_eq!(update.cancel_active, None);
        let disabled = MouseDispatchControls {
            configured: DispatchEnablement::Disabled,
            ..controls(1)
        };
        assert_eq!(q.advance(4, disabled).unwrap().cancel_active, Some(active));
        assert_eq!(q.advance(5, disabled).unwrap().cancel_active, None);
        assert_eq!(q.active().unwrap().token, active);
    }

    #[test]
    fn cancellation_preserves_running_occupancy_until_real_completion_and_revokes_old_epoch() {
        let mut q = queue();
        let first = token(
            q.offer(
                request(1, 1, MouseDispatchClass::Configured),
                0,
                controls(1),
            )
            .unwrap(),
        );
        q.offer(
            request(2, 1, MouseDispatchClass::Builtin { feature_id: 1 }),
            1,
            controls(1),
        )
        .unwrap();
        let cancelled = q.cancel();
        assert_eq!(cancelled.next_epoch, Some(2));
        assert_eq!(cancelled.update.cancel_active, Some(first));
        assert_eq!(
            cancelled.update.dropped[0].reason,
            MouseDispatchDropReason::Cancelled
        );
        assert!(q.active().unwrap().cancellation_requested);
        assert_eq!(
            q.offer(
                request(3, 1, MouseDispatchClass::Configured),
                2,
                controls(2)
            )
            .unwrap_err(),
            MouseDispatchError::EpochMismatch
        );
        let next = token(
            q.offer(
                request(3, 2, MouseDispatchClass::Configured),
                2,
                controls(2),
            )
            .unwrap(),
        );
        assert_eq!(q.active().unwrap().token, first);
        assert_eq!(
            q.complete(first, 3, controls(2))
                .unwrap()
                .started
                .unwrap()
                .token,
            next
        );
        assert_eq!(
            q.complete(first, 4, controls(2)).unwrap_err(),
            MouseDispatchError::CompletionMismatch
        );
    }

    #[test]
    fn target_context_invalidation_drops_waiting_and_only_requests_cancel_for_running() {
        let mut q = queue();
        let first = token(
            q.offer(
                request(1, 1, MouseDispatchClass::Configured),
                0,
                controls(1),
            )
            .unwrap(),
        );
        q.offer(
            MouseDispatchRequest {
                context_id: 101,
                ..request(2, 1, MouseDispatchClass::Builtin { feature_id: 1 })
            },
            1,
            controls(1),
        )
        .unwrap();
        let survivor = token(
            q.offer(
                request(3, 1, MouseDispatchClass::Configured),
                2,
                controls(1),
            )
            .unwrap(),
        );
        let revoked = q.invalidate_context(101, 3, controls(1)).unwrap();
        assert_eq!(revoked.cancel_active, Some(first));
        assert_eq!(
            revoked.dropped[0].reason,
            MouseDispatchDropReason::ContextInvalidated
        );
        assert_eq!(
            q.complete(first, 4, controls(1))
                .unwrap()
                .started
                .unwrap()
                .token,
            survivor
        );
    }

    #[test]
    fn each_builtin_has_its_own_switch_and_is_rechecked_before_start() {
        let mut q = queue();
        let first = token(
            q.offer(
                request(1, 1, MouseDispatchClass::Configured),
                0,
                controls(1),
            )
            .unwrap(),
        );
        q.offer(
            request(2, 1, MouseDispatchClass::Builtin { feature_id: 1 }),
            1,
            controls(1),
        )
        .unwrap();
        let survivor = token(
            q.offer(
                request(3, 1, MouseDispatchClass::Builtin { feature_id: 2 }),
                2,
                controls(1),
            )
            .unwrap(),
        );
        let switches = [
            BuiltinMouseSwitch {
                feature_id: 1,
                state: DispatchEnablement::Disabled,
            },
            BuiltinMouseSwitch {
                feature_id: 2,
                state: DispatchEnablement::Enabled,
            },
        ];
        let update = q
            .complete(
                first,
                3,
                MouseDispatchControls {
                    builtin: &switches,
                    ..controls(1)
                },
            )
            .unwrap();
        assert_eq!(update.dropped.len(), 1);
        assert_eq!(update.dropped[0].request.serial, 2);
        assert_eq!(update.dropped[0].reason, MouseDispatchDropReason::Disabled);
        assert_eq!(update.started.unwrap().token, survivor);
    }

    #[test]
    fn invalid_switch_maps_reject_before_mutation_and_max_capacity_has_bounded_cancel_receipts() {
        let mut q = MouseDispatchQueue::new(BusyMousePolicy::Delay {
            capacity: MAX_DELAYED_DISPATCHES,
            max_wait_ms: 1000,
        })
        .unwrap();
        let duplicate = [BuiltinMouseSwitch {
            feature_id: 1,
            state: DispatchEnablement::Enabled,
        }; 2];
        assert_eq!(
            q.offer(
                request(1, 1, MouseDispatchClass::Configured),
                0,
                MouseDispatchControls {
                    builtin: &duplicate,
                    ..controls(1)
                }
            )
            .unwrap_err(),
            MouseDispatchError::InvalidControls
        );
        assert!(q.active().is_none());
        let first = token(
            q.offer(
                request(1, 1, MouseDispatchClass::Configured),
                0,
                controls(1),
            )
            .unwrap(),
        );
        for serial in 2..=MAX_DELAYED_DISPATCHES as u64 + 1 {
            assert!(matches!(
                q.offer(
                    request(serial, 1, MouseDispatchClass::Configured),
                    serial,
                    controls(1)
                )
                .unwrap(),
                MouseDispatchAdmission::Delayed { .. }
            ));
        }
        assert_eq!(q.waiting_len(), MAX_DELAYED_DISPATCHES);
        assert_eq!(
            q.offer(
                request(100, 1, MouseDispatchClass::Configured),
                100,
                controls(1)
            )
            .unwrap(),
            MouseDispatchAdmission::Dropped(MouseDispatchDropReason::QueueFull)
        );
        let update = q.cancel().update;
        assert_eq!(update.dropped.len(), MAX_DELAYED_DISPATCHES);
        assert_eq!(update.cancel_active, Some(first));
        assert_eq!(q.waiting_len(), 0);
        assert_eq!(q.active().unwrap().token, first);
    }

    #[test]
    fn errors_do_not_release_active_or_silently_wrap_clock_deadline_and_token_ids() {
        assert!(matches!(
            MouseDispatchQueue::new(BusyMousePolicy::Delay {
                capacity: 65,
                max_wait_ms: 1
            }),
            Err(MouseDispatchError::InvalidPolicy)
        ));
        assert!(matches!(
            MouseDispatchQueue::new(BusyMousePolicy::Delay {
                capacity: 1,
                max_wait_ms: 0
            }),
            Err(MouseDispatchError::InvalidPolicy)
        ));
        let mut q = queue();
        let first = token(
            q.offer(
                request(1, 1, MouseDispatchClass::Configured),
                100,
                controls(1),
            )
            .unwrap(),
        );
        assert_eq!(
            q.advance(99, controls(1)).unwrap_err(),
            MouseDispatchError::ClockWentBackward
        );
        assert_eq!(
            q.complete(first + 1, 101, controls(1)).unwrap_err(),
            MouseDispatchError::CompletionMismatch
        );
        assert_eq!(q.active().unwrap().token, first);
        assert_eq!(
            q.offer(
                request(2, 1, MouseDispatchClass::Configured),
                u64::MAX,
                controls(1)
            )
            .unwrap_err(),
            MouseDispatchError::DeadlineOverflow
        );
        q.complete(first, u64::MAX, controls(1)).unwrap();
        q.next_token = Some(u64::MAX);
        let last = token(
            q.offer(
                request(3, 1, MouseDispatchClass::Configured),
                u64::MAX,
                controls(1),
            )
            .unwrap(),
        );
        assert_eq!(last, u64::MAX);
        q.complete(last, u64::MAX, controls(1)).unwrap();
        assert_eq!(
            q.offer(
                request(4, 1, MouseDispatchClass::Configured),
                u64::MAX,
                controls(1)
            )
            .unwrap_err(),
            MouseDispatchError::TokenExhausted
        );
    }

    #[test]
    fn epoch_exhaustion_closes_admission_but_keeps_receipts_and_allows_real_cleanup() {
        let mut q = queue();
        q.epoch = u64::MAX;
        let first = token(
            q.offer(
                request(1, u64::MAX, MouseDispatchClass::Configured),
                0,
                controls(u64::MAX),
            )
            .unwrap(),
        );
        q.offer(
            request(2, u64::MAX, MouseDispatchClass::Configured),
            1,
            controls(u64::MAX),
        )
        .unwrap();
        let stopped = q.cancel();
        assert_eq!(stopped.next_epoch, None);
        assert_eq!(stopped.update.cancel_active, Some(first));
        assert_eq!(stopped.update.dropped.len(), 1);
        assert_eq!(
            q.offer(
                request(3, u64::MAX, MouseDispatchClass::Configured),
                2,
                controls(u64::MAX)
            )
            .unwrap_err(),
            MouseDispatchError::Closed
        );
        assert!(
            q.complete(first, 3, controls(u64::MAX))
                .unwrap()
                .started
                .is_none()
        );
        assert!(q.active().is_none());
    }
}
