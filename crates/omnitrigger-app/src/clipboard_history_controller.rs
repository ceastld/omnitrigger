//! 历史监听的纯生命周期控制与追加许可；默认关闭，不读取剪贴板或调用存储。
//!
//! 接线层只执行返回的命令。存储回调应在后台经 [`HistoryControl::commit`] 执行；
//! 同一互斥门使停用确认与写入提交串行，停用返回后旧 session 不得再进入存储。
//! UI 在停用确认前显示“关闭中”，不能先宣称已关闭。回调不能重入控制器。

use std::fmt;
use std::sync::Mutex;

const MAX_UNITS: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionKey {
    pub generation: u64,
    pub listener_session: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitialCapturePolicy {
    /// 启动只记序号基线，不能读取、导入或追加已经存在的正文。
    FutureNotificationsOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Stopped,
    Starting { generation: u64, revoked: bool },
    Running { key: SessionKey },
    Stopping { key: SessionKey },
    CleanupFailed { key: SessionKey },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    StartFailed,
    StopFailed,
    UnexpectedStop,
    CounterExhausted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreFailure {
    Unavailable,
    Conflict,
    Cancelled,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Start {
        generation: u64,
        initial: InitialCapturePolicy,
    },
    CancelStart {
        generation: u64,
    },
    Stop {
        key: SessionKey,
    },
    Report(Failure),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropReason {
    InactiveSession,
    DuplicateSequence,
    AppendPending,
    InvalidText,
}

/// 仅接线层传入；Debug 不输出正文。
pub struct Observation {
    pub key: SessionKey,
    pub sequence: u32,
    pub text: String,
}

impl fmt::Debug for Observation {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output
            .debug_struct("Observation")
            .field("key", &self.key)
            .field("sequence", &self.sequence)
            .field("utf16_units", &self.text.encode_utf16().count())
            .finish()
    }
}

/// 许可绑定创建它的控制器；接线层不能构造或修改，复制也不会重复执行同一追加。
#[derive(Clone)]
pub struct AppendRequest {
    origin: std::sync::Arc<()>,
    key: SessionKey,
    ticket: u64,
    sequence: u32,
    text: String,
}

impl fmt::Debug for AppendRequest {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output
            .debug_struct("AppendRequest")
            .field("key", &self.key)
            .field("ticket", &self.ticket)
            .field("sequence", &self.sequence)
            .field("utf16_units", &self.text.encode_utf16().count())
            .finish()
    }
}

#[derive(Debug)]
pub enum Event {
    Enable,
    Disable,
    Started {
        generation: u64,
        listener_session: u64,
        baseline_sequence: u32,
    },
    StartFailed {
        generation: u64,
    },
    Stopped {
        key: SessionKey,
    },
    StopFailed {
        key: SessionKey,
    },
    RetryCleanup,
    Observed(Observation),
}

#[derive(Debug, Default)]
pub struct Decision {
    pub commands: Vec<Command>,
    pub append: Option<AppendRequest>,
    pub dropped: Option<DropReason>,
}

#[derive(Clone, Copy)]
struct Pending {
    key: SessionKey,
    ticket: u64,
    sequence: u32,
}

/// 无时间、磁盘或 Win32 依赖；同一实例最多一个启动/运行/退出资源和一个追加许可。
pub struct HistoryController {
    origin: std::sync::Arc<()>,
    status: Status,
    desired: bool,
    next_generation: u64,
    next_ticket: u64,
    previous_sequence: u32,
    pending: Option<Pending>,
}

impl Default for HistoryController {
    fn default() -> Self {
        Self {
            origin: std::sync::Arc::new(()),
            status: Status::Stopped,
            desired: false,
            next_generation: 1,
            next_ticket: 1,
            previous_sequence: 0,
            pending: None,
        }
    }
}

impl HistoryController {
    pub fn status(&self) -> Status {
        self.status
    }
    #[cfg(test)]
    pub fn desired_enabled(&self) -> bool {
        self.desired
    }

    fn start(&mut self, decision: &mut Decision) {
        let Some(next) = self.next_generation.checked_add(1) else {
            self.desired = false;
            decision
                .commands
                .push(Command::Report(Failure::CounterExhausted));
            return;
        };
        let generation = self.next_generation;
        self.next_generation = next;
        self.status = Status::Starting {
            generation,
            revoked: false,
        };
        decision.commands.push(Command::Start {
            generation,
            initial: InitialCapturePolicy::FutureNotificationsOnly,
        });
    }

    pub fn apply(&mut self, event: Event) -> Decision {
        let mut decision = Decision::default();
        match event {
            Event::Enable => {
                self.desired = true;
                if self.status == Status::Stopped {
                    self.start(&mut decision);
                }
            }
            Event::Disable => {
                self.desired = false;
                self.pending = None;
                match self.status {
                    Status::Starting {
                        generation,
                        revoked: false,
                    } => {
                        self.status = Status::Starting {
                            generation,
                            revoked: true,
                        };
                        decision.commands.push(Command::CancelStart { generation });
                    }
                    Status::Running { key } => {
                        self.status = Status::Stopping { key };
                        decision.commands.push(Command::Stop { key });
                    }
                    _ => {}
                }
            }
            Event::Started {
                generation,
                listener_session,
                baseline_sequence,
            } => {
                let key = SessionKey {
                    generation,
                    listener_session,
                };
                match self.status {
                    Status::Starting {
                        generation: expected,
                        revoked,
                    } if generation == expected => {
                        if !revoked && self.desired {
                            self.previous_sequence = baseline_sequence;
                            self.status = Status::Running { key };
                        } else {
                            self.status = Status::Stopping { key };
                            decision.commands.push(Command::Stop { key });
                        }
                    }
                    Status::Running { key: current } if current == key => {}
                    _ => decision.commands.push(Command::Stop { key }),
                }
            }
            Event::StartFailed { generation } => {
                if let Status::Starting {
                    generation: expected,
                    revoked,
                } = self.status
                {
                    if generation == expected {
                        self.status = Status::Stopped;
                        self.pending = None;
                        if revoked && self.desired {
                            // 这是撤销后用户再次启用的请求，不是对失败启动的自动重试。
                            self.start(&mut decision);
                        } else {
                            self.desired = false;
                            if !revoked {
                                decision
                                    .commands
                                    .push(Command::Report(Failure::StartFailed));
                            }
                        }
                    }
                }
            }
            Event::Stopped { key } => match self.status {
                Status::Stopping { key: expected } | Status::CleanupFailed { key: expected }
                    if key == expected =>
                {
                    self.status = Status::Stopped;
                    self.pending = None;
                    if self.desired {
                        self.start(&mut decision);
                    }
                }
                Status::Running { key: expected } if key == expected => {
                    self.status = Status::Stopped;
                    self.desired = false;
                    self.pending = None;
                    decision
                        .commands
                        .push(Command::Report(Failure::UnexpectedStop));
                }
                _ => {}
            },
            Event::StopFailed { key } => {
                if self.status == (Status::Stopping { key }) {
                    self.status = Status::CleanupFailed { key };
                    self.desired = false;
                    self.pending = None;
                    decision.commands.push(Command::Report(Failure::StopFailed));
                }
            }
            Event::RetryCleanup => {
                if let Status::CleanupFailed { key } = self.status {
                    self.status = Status::Stopping { key };
                    decision.commands.push(Command::Stop { key });
                }
            }
            Event::Observed(value) => {
                if !self.desired || self.status != (Status::Running { key: value.key }) {
                    decision.dropped = Some(DropReason::InactiveSession);
                } else if value.sequence == self.previous_sequence {
                    decision.dropped = Some(DropReason::DuplicateSequence);
                } else if self.pending.is_some() {
                    decision.dropped = Some(DropReason::AppendPending);
                } else if value.text.contains('\0') || value.text.encode_utf16().count() > MAX_UNITS
                {
                    decision.dropped = Some(DropReason::InvalidText);
                } else if let Some(next) = self.next_ticket.checked_add(1) {
                    let ticket = self.next_ticket;
                    self.next_ticket = next;
                    self.previous_sequence = value.sequence;
                    self.pending = Some(Pending {
                        key: value.key,
                        ticket,
                        sequence: value.sequence,
                    });
                    decision.append = Some(AppendRequest {
                        origin: self.origin.clone(),
                        key: value.key,
                        ticket,
                        sequence: value.sequence,
                        text: value.text,
                    });
                } else {
                    self.desired = false;
                    self.pending = None;
                    self.status = Status::Stopping { key: value.key };
                    decision.commands.push(Command::Stop { key: value.key });
                    decision
                        .commands
                        .push(Command::Report(Failure::CounterExhausted));
                }
            }
        }
        decision
    }

    fn permits(&self, request: &AppendRequest) -> bool {
        std::sync::Arc::ptr_eq(&self.origin, &request.origin)
            && self.desired
            && self.status == (Status::Running { key: request.key })
            && self.pending.is_some_and(|pending| {
                pending.key == request.key
                    && pending.ticket == request.ticket
                    && pending.sequence == request.sequence
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlError {
    Poisoned,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitDisposition {
    Stored,
    Discarded,
    Failed(StoreFailure),
}

/// 纯同步门；内部不读文件。调用方在后台运行，不能在 GUI/Hook 回调等待此锁。
#[derive(Default)]
pub struct HistoryControl {
    controller: Mutex<HistoryController>,
}

impl HistoryControl {
    pub fn apply(&self, event: Event) -> Result<Decision, ControlError> {
        self.controller
            .lock()
            .map_err(|_| ControlError::Poisoned)
            .map(|mut controller| controller.apply(event))
    }

    pub fn status(&self) -> Result<Status, ControlError> {
        self.controller
            .lock()
            .map_err(|_| ControlError::Poisoned)
            .map(|controller| controller.status())
    }

    /// 必须覆盖存储的真实事务提交，不能仅把写入排队后返回 Ok。
    /// 许可在调用前消耗；失败和异常均不自动重试。异常会毒化门，接线层须停机并保留错误。
    pub fn commit(
        &self,
        request: &AppendRequest,
        append: impl FnOnce(&str) -> Result<(), StoreFailure>,
    ) -> Result<CommitDisposition, ControlError> {
        let mut controller = self.controller.lock().map_err(|_| ControlError::Poisoned)?;
        if !controller.permits(request) {
            return Ok(CommitDisposition::Discarded);
        }
        controller.pending = None;
        Ok(match append(&request.text) {
            Ok(()) => CommitDisposition::Stored,
            Err(error) => CommitDisposition::Failed(error),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running() -> (HistoryControl, SessionKey) {
        let control = HistoryControl::default();
        assert!(matches!(
            control.apply(Event::Enable).unwrap().commands.as_slice(),
            [Command::Start {
                generation: 1,
                initial: InitialCapturePolicy::FutureNotificationsOnly
            }]
        ));
        control
            .apply(Event::Started {
                generation: 1,
                listener_session: 7,
                baseline_sequence: 10,
            })
            .unwrap();
        (
            control,
            SessionKey {
                generation: 1,
                listener_session: 7,
            },
        )
    }
    fn observed(key: SessionKey, sequence: u32, text: &str) -> Event {
        Event::Observed(Observation {
            key,
            sequence,
            text: text.into(),
        })
    }

    #[test]
    fn default_does_nothing_and_start_never_requests_existing_body() {
        let mut control = HistoryController::default();
        assert_eq!(control.status(), Status::Stopped);
        assert!(!control.desired_enabled());
        assert!(control.apply(Event::Disable).commands.is_empty());
        let first = control.apply(Event::Enable);
        assert!(first.append.is_none());
        assert_eq!(
            first.commands,
            [Command::Start {
                generation: 1,
                initial: InitialCapturePolicy::FutureNotificationsOnly
            }]
        );
        assert!(control.apply(Event::Enable).commands.is_empty());
    }

    #[test]
    fn baseline_is_not_imported_but_empty_and_whitespace_future_text_are_valid() {
        for text in ["", " \r\n\t ", "合成😀"] {
            let (control, key) = running();
            assert_eq!(
                control
                    .apply(observed(key, 10, "synthetic old body"))
                    .unwrap()
                    .dropped,
                Some(DropReason::DuplicateSequence)
            );
            let request = control
                .apply(observed(key, 11, text))
                .unwrap()
                .append
                .unwrap();
            let mut saved = None;
            assert_eq!(
                control
                    .commit(&request, |value| {
                        saved = Some(value.to_owned());
                        Ok(())
                    })
                    .unwrap(),
                CommitDisposition::Stored
            );
            assert_eq!(saved.as_deref(), Some(text));
        }
    }

    #[test]
    fn disable_invalidates_queued_append_before_store_callback() {
        let (control, key) = running();
        let request = control
            .apply(observed(key, 11, "synthetic"))
            .unwrap()
            .append
            .unwrap();
        assert_eq!(
            control.apply(Event::Disable).unwrap().commands,
            [Command::Stop { key }]
        );
        assert_eq!(
            control
                .commit(&request, |_| panic!("旧session不能调用存储"))
                .unwrap(),
            CommitDisposition::Discarded
        );
        assert_eq!(
            control
                .apply(observed(key, 12, "synthetic late"))
                .unwrap()
                .dropped,
            Some(DropReason::InactiveSession)
        );
    }

    #[test]
    fn append_ticket_is_consumed_once_even_after_failure() {
        for result in [
            Ok(()),
            Err(StoreFailure::Conflict),
            Err(StoreFailure::Cancelled),
        ] {
            let (control, key) = running();
            let request = control
                .apply(observed(key, 11, "synthetic"))
                .unwrap()
                .append
                .unwrap();
            let duplicate = request.clone();
            control.commit(&request, |_| result).unwrap();
            assert_eq!(
                control
                    .commit(&duplicate, |_| panic!("不能重试已消费许可"))
                    .unwrap(),
                CommitDisposition::Discarded
            );
            assert_eq!(
                control
                    .apply(observed(key, 11, "synthetic"))
                    .unwrap()
                    .dropped,
                Some(DropReason::DuplicateSequence)
            );
        }
    }

    #[test]
    fn only_one_append_is_pending_and_invalid_text_is_never_forwarded() {
        let (control, key) = running();
        for text in ["bad\0body".into(), "x".repeat(MAX_UNITS + 1)] {
            assert_eq!(
                control.apply(observed(key, 11, &text)).unwrap().dropped,
                Some(DropReason::InvalidText)
            );
        }
        let request = control
            .apply(observed(key, 11, "synthetic"))
            .unwrap()
            .append
            .unwrap();
        assert_eq!(
            control
                .apply(observed(key, 12, "synthetic newer"))
                .unwrap()
                .dropped,
            Some(DropReason::AppendPending)
        );
        control.commit(&request, |_| Ok(())).unwrap();
        assert!(control
            .apply(observed(key, 12, "synthetic newer"))
            .unwrap()
            .append
            .is_some());
    }

    #[test]
    fn disable_enable_during_start_never_reuses_revoked_generation() {
        let mut control = HistoryController::default();
        control.apply(Event::Enable);
        assert_eq!(
            control.apply(Event::Disable).commands,
            [Command::CancelStart { generation: 1 }]
        );
        assert!(control.apply(Event::Enable).commands.is_empty());
        let old = SessionKey {
            generation: 1,
            listener_session: 7,
        };
        assert_eq!(
            control
                .apply(Event::Started {
                    generation: 1,
                    listener_session: 7,
                    baseline_sequence: 10
                })
                .commands,
            [Command::Stop { key: old }]
        );
        assert_eq!(
            control.apply(Event::Stopped { key: old }).commands,
            [Command::Start {
                generation: 2,
                initial: InitialCapturePolicy::FutureNotificationsOnly
            }]
        );
    }

    #[test]
    fn ordinary_start_failure_never_spawns_an_automatic_retry() {
        let mut control = HistoryController::default();
        control.apply(Event::Enable);
        assert_eq!(
            control.apply(Event::StartFailed { generation: 1 }).commands,
            [Command::Report(Failure::StartFailed)]
        );
        assert_eq!(control.status(), Status::Stopped);
        assert!(!control.desired_enabled());
        assert!(control
            .apply(Event::StartFailed { generation: 1 })
            .commands
            .is_empty());
        assert!(matches!(
            control.apply(Event::Enable).commands.as_slice(),
            [Command::Start { generation: 2, .. }]
        ));
    }

    #[test]
    fn cleanup_failure_blocks_new_start_until_explicit_retry_and_ack() {
        let (control, key) = running();
        control.apply(Event::Disable).unwrap();
        assert_eq!(
            control.apply(Event::StopFailed { key }).unwrap().commands,
            [Command::Report(Failure::StopFailed)]
        );
        assert!(control.apply(Event::Enable).unwrap().commands.is_empty());
        assert_eq!(
            control.apply(Event::RetryCleanup).unwrap().commands,
            [Command::Stop { key }]
        );
        assert!(matches!(
            control
                .apply(Event::Stopped { key })
                .unwrap()
                .commands
                .as_slice(),
            [Command::Start { generation: 2, .. }]
        ));
    }

    #[test]
    fn stale_started_is_closed_without_replacing_running_session() {
        let (control, key) = running();
        let stale = SessionKey {
            generation: 99,
            listener_session: 7,
        };
        assert_eq!(
            control
                .apply(Event::Started {
                    generation: 99,
                    listener_session: 7,
                    baseline_sequence: 0
                })
                .unwrap()
                .commands,
            [Command::Stop { key: stale }]
        );
        assert_eq!(control.status().unwrap(), Status::Running { key });
        assert_eq!(
            control
                .apply(observed(stale, 11, "synthetic"))
                .unwrap()
                .dropped,
            Some(DropReason::InactiveSession)
        );
    }

    #[test]
    fn unexpected_exit_revokes_tickets_and_does_not_install_a_watchdog() {
        let (control, key) = running();
        let request = control
            .apply(observed(key, 11, "synthetic"))
            .unwrap()
            .append
            .unwrap();
        assert_eq!(
            control.apply(Event::Stopped { key }).unwrap().commands,
            [Command::Report(Failure::UnexpectedStop)]
        );
        assert_eq!(
            control
                .commit(&request, |_| panic!("退出后不得提交"))
                .unwrap(),
            CommitDisposition::Discarded
        );
        assert_eq!(control.status().unwrap(), Status::Stopped);
    }

    #[test]
    fn counters_fail_closed_without_reusing_generation_or_append_ticket() {
        let mut controller = HistoryController {
            next_generation: u64::MAX,
            ..Default::default()
        };
        assert_eq!(
            controller.apply(Event::Enable).commands,
            [Command::Report(Failure::CounterExhausted)]
        );
        assert_eq!(controller.status(), Status::Stopped);
        let mut controller = HistoryController::default();
        controller.apply(Event::Enable);
        controller.apply(Event::Started {
            generation: 1,
            listener_session: 7,
            baseline_sequence: 0,
        });
        controller.next_ticket = u64::MAX;
        let key = SessionKey {
            generation: 1,
            listener_session: 7,
        };
        assert_eq!(
            controller.apply(observed(key, 1, "synthetic")).commands,
            [
                Command::Stop { key },
                Command::Report(Failure::CounterExhausted)
            ]
        );
    }

    #[test]
    fn store_panic_poison_is_fail_closed_and_cannot_retry_a_possible_commit() {
        let (control, key) = running();
        let request = control
            .apply(observed(key, 11, "synthetic"))
            .unwrap()
            .append
            .unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            control.commit(&request, |_| panic!("合成存储异常"))
        }));
        assert!(result.is_err());
        assert_eq!(
            control.commit(&request, |_| Ok(())),
            Err(ControlError::Poisoned)
        );
        assert!(matches!(
            control.apply(Event::Disable),
            Err(ControlError::Poisoned)
        ));
    }

    #[test]
    fn diagnostics_do_not_echo_observed_or_pending_body() {
        let (control, key) = running();
        let value = observed(key, 11, "synthetic-private-正文");
        assert!(!format!("{value:?}").contains("synthetic-private-正文"));
        let decision = control.apply(value).unwrap();
        assert!(!format!("{decision:?}").contains("synthetic-private-正文"));
    }

    #[test]
    fn disable_ack_waits_for_existing_commit_and_prevents_all_later_old_writes() {
        use std::sync::{mpsc, Arc};
        use std::time::Duration;
        let (control, key) = running();
        let control = Arc::new(control);
        let request = control
            .apply(observed(key, 11, "synthetic"))
            .unwrap()
            .append
            .unwrap();
        let retry = request.clone();
        let (entered, has_entered) = mpsc::sync_channel(1);
        let (release, released) = mpsc::sync_channel(1);
        let writer_control = control.clone();
        let writer = std::thread::spawn(move || {
            writer_control.commit(&request, |_| {
                entered.send(()).unwrap();
                released.recv_timeout(Duration::from_secs(2)).unwrap();
                Ok(())
            })
        });
        has_entered.recv_timeout(Duration::from_secs(2)).unwrap();
        let (disable_requested, requested) = mpsc::sync_channel(1);
        let (disable_done, done) = mpsc::sync_channel(1);
        let disable_control = control.clone();
        let disabling = std::thread::spawn(move || {
            disable_requested.send(()).unwrap();
            let decision = disable_control.apply(Event::Disable).unwrap();
            disable_done.send(decision).unwrap();
        });
        requested.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(done.try_recv(), Err(mpsc::TryRecvError::Empty)));
        release.send(()).unwrap();
        assert_eq!(writer.join().unwrap().unwrap(), CommitDisposition::Stored);
        assert_eq!(
            done.recv_timeout(Duration::from_secs(2)).unwrap().commands,
            [Command::Stop { key }]
        );
        disabling.join().unwrap();
        assert_eq!(
            control
                .commit(&retry, |_| panic!("停用确认后不能提交旧项"))
                .unwrap(),
            CommitDisposition::Discarded
        );
    }

    #[test]
    fn restarting_cannot_adopt_old_records_even_when_native_session_number_is_reused() {
        let (control, old) = running();
        let request = control
            .apply(observed(old, 11, "synthetic old"))
            .unwrap()
            .append
            .unwrap();
        control.apply(Event::Disable).unwrap();
        control.apply(Event::Enable).unwrap();
        assert!(matches!(
            control
                .apply(Event::Stopped { key: old })
                .unwrap()
                .commands
                .as_slice(),
            [Command::Start { generation: 2, .. }]
        ));
        control
            .apply(Event::Started {
                generation: 2,
                listener_session: old.listener_session,
                baseline_sequence: 12,
            })
            .unwrap();
        assert_eq!(
            control
                .apply(observed(old, 13, "synthetic late"))
                .unwrap()
                .dropped,
            Some(DropReason::InactiveSession)
        );
        assert_eq!(
            control
                .commit(&request, |_| panic!("旧代许可不能复活"))
                .unwrap(),
            CommitDisposition::Discarded
        );
        let current = SessionKey {
            generation: 2,
            listener_session: old.listener_session,
        };
        assert!(control
            .apply(observed(current, 13, "synthetic new"))
            .unwrap()
            .append
            .is_some());
    }

    #[test]
    fn foreign_owner_ticket_cannot_consume_recreated_controller_pending() {
        let (old, old_key) = running();
        let stale = old
            .apply(observed(old_key, 11, "合成旧owner正文"))
            .unwrap()
            .append
            .unwrap();
        old.apply(Event::Disable).unwrap();
        old.apply(Event::Stopped { key: old_key }).unwrap();
        drop(old);

        let (current, current_key) = running();
        assert_eq!(current_key, old_key);
        let own = current
            .apply(observed(current_key, 11, "合成新owner正文"))
            .unwrap()
            .append
            .unwrap();
        assert_eq!(
            current
                .commit(&stale, |_| panic!("其他owner的许可不能进入存储"))
                .unwrap(),
            CommitDisposition::Discarded
        );
        let mut stored = Vec::new();
        assert_eq!(
            current
                .commit(&own, |text| {
                    stored.push(text.to_owned());
                    Ok(())
                })
                .unwrap(),
            CommitDisposition::Stored
        );
        assert_eq!(stored, ["合成新owner正文"]);
        assert_eq!(
            current
                .commit(&own, |_| panic!("同一许可仍只能消费一次"))
                .unwrap(),
            CommitDisposition::Discarded
        );
        assert!(!format!("{own:?}").contains("合成新owner正文"));
    }

    #[test]
    fn sequence_wrap_is_equality_only_and_text_boundary_uses_utf16() {
        let control = HistoryControl::default();
        control.apply(Event::Enable).unwrap();
        control
            .apply(Event::Started {
                generation: 1,
                listener_session: 7,
                baseline_sequence: u32::MAX,
            })
            .unwrap();
        let key = SessionKey {
            generation: 1,
            listener_session: 7,
        };
        let text = "😀".repeat(MAX_UNITS / 2);
        let request = control
            .apply(observed(key, 0, &text))
            .unwrap()
            .append
            .unwrap();
        assert_eq!(
            control
                .commit(&request, |value| {
                    assert_eq!(value.encode_utf16().count(), MAX_UNITS);
                    Ok(())
                })
                .unwrap(),
            CommitDisposition::Stored
        );
        assert_eq!(
            control
                .apply(observed(key, 1, &(text + "x")))
                .unwrap()
                .dropped,
            Some(DropReason::InvalidText)
        );
    }

    #[test]
    fn revoked_start_failure_releases_resource_slot_for_a_later_user_enable() {
        let mut controller = HistoryController::default();
        controller.apply(Event::Enable);
        controller.apply(Event::Disable);
        controller.apply(Event::Enable);
        assert_eq!(
            controller.status(),
            Status::Starting {
                generation: 1,
                revoked: true
            }
        );
        assert!(matches!(
            controller
                .apply(Event::StartFailed { generation: 1 })
                .commands
                .as_slice(),
            [Command::Start { generation: 2, .. }]
        ));
    }
}
