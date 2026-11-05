//! 仅供应用后台执行器调用的历史生命周期适配。构造不启用监听，不创建线程或任务。
//! 每次poll至多处理一项；存储回调必须完成真实同步事务，不能只排队后返回成功。

use crate::clipboard_history_controller::{
    Command, CommitDisposition, ControlError, Decision, DropReason, Event, Failure, HistoryControl,
    InitialCapturePolicy, Observation, SessionKey, Status, StoreFailure,
};
use crate::clipboard_history_store::{ClipboardHistoryStore, HistoryError};
use std::collections::VecDeque;
use std::fmt;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

pub struct ObservedText {
    pub sequence: u32,
    pub text: String,
}
impl fmt::Debug for ObservedText {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output
            .debug_struct("ObservedText")
            .field("sequence", &self.sequence)
            .field("utf16_units", &self.text.encode_utf16().count())
            .finish()
    }
}
pub enum ListenerPoll {
    Empty,
    Text(ObservedText),
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackgroundError {
    InvalidState,
    ListenerFailed,
    CleanupFailed,
    ControlPoisoned,
}

/// 句柄必须拥有自己的原生资源，stop完成join，Drop兜底。此接口不允许选取其他进程资源。
pub trait BackgroundListener: Send {
    fn session(&self) -> u64;
    fn baseline_sequence(&self) -> u32;
    fn try_next(&mut self) -> Result<ListenerPoll, BackgroundError>;
    fn stop(&mut self) -> Result<(), BackgroundError>;
}

pub trait ListenerFactory: Send {
    /// 只接受未来通知策略；失败返回前必须清理部分启动资源。不得读取旧正文。
    fn start(
        &mut self,
        initial: InitialCapturePolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn BackgroundListener>, BackgroundError>;
}

pub trait BackgroundStore: Send {
    /// 应把cancel检查交给store事务；错误只返回固定分类，保留unknown，不回显正文。
    fn append(&mut self, text: &str, cancel: &Cancellation) -> Result<(), StoreFailure>;
}

pub struct ClipboardHistoryWriter {
    store: Arc<ClipboardHistoryStore>,
}
impl ClipboardHistoryWriter {
    pub fn new(store: Arc<ClipboardHistoryStore>) -> Self {
        Self { store }
    }
}
fn store_failure(error: HistoryError) -> StoreFailure {
    if error.effect_state == "unknown" {
        return StoreFailure::Unknown;
    }
    match error.code {
        "history_conflict" => StoreFailure::Conflict,
        "cancelled" | "timeout" => StoreFailure::Cancelled,
        "history_busy" | "history_unavailable" => StoreFailure::Unavailable,
        _ if error.effect_state == "not_started" => StoreFailure::Unavailable,
        _ => StoreFailure::Unknown,
    }
}
impl BackgroundStore for ClipboardHistoryWriter {
    fn append(&mut self, text: &str, cancel: &Cancellation) -> Result<(), StoreFailure> {
        self.store
            .append(text, || {
                if cancel.is_cancelled() {
                    Err(HistoryError::new("cancelled"))
                } else {
                    Ok(())
                }
            })
            .map(|_receipt| ())
            .map_err(store_failure)
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use omnitrigger_windows::{ClipboardHistoryListener, HistoryText};
    use std::sync::mpsc::{Receiver, TryRecvError};

    /// 只使用当前应用自己的helper入口，不接受脚本/历史正文提供的路径。
    #[derive(Default)]
    pub struct NativeHistoryFactory {
        worker_budgets: Option<omnitrigger_windows::HistoryWorkerBudgets>,
    }
    struct Listener {
        listener: ClipboardHistoryListener,
        results: Receiver<HistoryText>,
        baseline: u32,
    }
    impl ListenerFactory for NativeHistoryFactory {
        fn start(
            &mut self,
            initial: InitialCapturePolicy,
            cancel: &Cancellation,
        ) -> Result<Box<dyn BackgroundListener>, BackgroundError> {
            let InitialCapturePolicy::FutureNotificationsOnly = initial;
            if cancel.is_cancelled() {
                return Err(BackgroundError::ListenerFailed);
            }
            let executable =
                std::env::current_exe().map_err(|_| BackgroundError::ListenerFailed)?;
            let (listener, results) = match self.worker_budgets {
                None => ClipboardHistoryListener::start(&executable),
                Some(budgets) => ClipboardHistoryListener::start_persistent(&executable, budgets),
            }
            .map_err(|_| BackgroundError::ListenerFailed)?;
            // 更保守的启用交接基线：只读序号元数据；交接期间的变更可能被跳过，不补读旧正文。
            let baseline = omnitrigger_windows::clipboard_sequence_number();
            Ok(Box::new(Listener {
                listener,
                results,
                baseline,
            }))
        }
    }
    impl BackgroundListener for Listener {
        fn session(&self) -> u64 {
            self.listener.session()
        }
        fn baseline_sequence(&self) -> u32 {
            self.baseline
        }
        fn try_next(&mut self) -> Result<ListenerPoll, BackgroundError> {
            match self.results.try_recv() {
                Ok(record) if record.session == self.listener.session() => {
                    Ok(ListenerPoll::Text(ObservedText {
                        sequence: record.sequence_number,
                        text: record.text,
                    }))
                }
                Ok(_) => Err(BackgroundError::ListenerFailed),
                Err(TryRecvError::Empty) => Ok(ListenerPoll::Empty),
                Err(TryRecvError::Disconnected) => Ok(ListenerPoll::Closed),
            }
        }
        fn stop(&mut self) -> Result<(), BackgroundError> {
            self.listener
                .stop()
                .map(|_stats| ())
                .map_err(|_| BackgroundError::CleanupFailed)
        }
    }
    impl HistoryBackground<NativeHistoryFactory, ClipboardHistoryWriter> {
        /// 构造仍默认关闭；预算只影响明确启用后的持续helper。
        pub fn for_store_with_worker(
            store: Arc<ClipboardHistoryStore>,
            budgets: omnitrigger_windows::HistoryWorkerBudgets,
        ) -> Self {
            let mut background = Self::for_store(store);
            background.factory.worker_budgets = Some(budgets);
            background
        }

        /// 仅保存既有私有store引用；默认关闭，真正启用必须在后台显式调用enable。
        pub fn for_store(store: Arc<ClipboardHistoryStore>) -> Self {
            Self::new(
                NativeHistoryFactory::default(),
                ClipboardHistoryWriter::new(store),
            )
        }
    }
}

#[derive(Debug)]
pub struct BackgroundStep {
    pub status: Status,
    pub reports: Vec<Failure>,
    pub dropped: Option<DropReason>,
    pub commit: Option<CommitDisposition>,
}

/// 所有方法由同一后台执行器串行调用；UI只保留Cancellation，不能在GUI/Hook等待方法。
pub struct HistoryBackground<F: ListenerFactory, S: BackgroundStore> {
    control: HistoryControl,
    factory: F,
    store: S,
    active: Option<(SessionKey, Box<dyn BackgroundListener>)>,
    cancellation: Cancellation,
}

impl<F: ListenerFactory, S: BackgroundStore> HistoryBackground<F, S> {
    pub fn new(factory: F, store: S) -> Self {
        let cancellation = Cancellation::default();
        cancellation.cancel();
        Self {
            control: HistoryControl::default(),
            factory,
            store,
            active: None,
            cancellation,
        }
    }
    pub fn status(&self) -> Result<Status, BackgroundError> {
        self.control
            .status()
            .map_err(|_| BackgroundError::ControlPoisoned)
    }
    fn decision(&self, event: Event) -> Result<Decision, BackgroundError> {
        self.control
            .apply(event)
            .map_err(|_| BackgroundError::ControlPoisoned)
    }
    fn execute(&mut self, decision: Decision) -> Result<BackgroundStep, BackgroundError> {
        let mut reports = vec![];
        let mut commands: VecDeque<Command> = decision.commands.into();
        while let Some(command) = commands.pop_front() {
            match command {
                Command::Report(failure) => reports.push(failure),
                Command::CancelStart { .. } => self.cancellation.cancel(),
                Command::Start {
                    generation,
                    initial,
                } => {
                    if self.active.is_some() {
                        return Err(BackgroundError::InvalidState);
                    }
                    let started = self.factory.start(initial, &self.cancellation);
                    if self.cancellation.is_cancelled() {
                        commands.extend(self.decision(Event::Disable)?.commands);
                    }
                    match started {
                        Ok(listener) => {
                            let key = SessionKey {
                                generation,
                                listener_session: listener.session(),
                            };
                            let baseline_sequence = listener.baseline_sequence();
                            self.active = Some((key, listener));
                            commands.extend(
                                self.decision(Event::Started {
                                    generation,
                                    listener_session: key.listener_session,
                                    baseline_sequence,
                                })?
                                .commands,
                            );
                        }
                        Err(_) => commands
                            .extend(self.decision(Event::StartFailed { generation })?.commands),
                    }
                }
                Command::Stop { key } => {
                    self.cancellation.cancel();
                    if let Some((current, listener)) = self.active.as_mut() {
                        if *current != key {
                            return Err(BackgroundError::InvalidState);
                        }
                        if listener.stop().is_err() {
                            commands.extend(self.decision(Event::StopFailed { key })?.commands);
                            continue;
                        }
                        self.active.take();
                    }
                    commands.extend(self.decision(Event::Stopped { key })?.commands);
                }
            }
        }
        Ok(BackgroundStep {
            status: self.status()?,
            reports,
            dropped: decision.dropped,
            commit: None,
        })
    }
    /// 每次显式启用传入新的token；调用方在排队前持有其clone，可取消正在启动或提交的一代。
    pub fn enable(
        &mut self,
        cancellation: Cancellation,
    ) -> Result<BackgroundStep, BackgroundError> {
        if self.status()? != Status::Stopped || self.active.is_some() {
            return Err(BackgroundError::InvalidState);
        }
        self.cancellation = cancellation;
        if self.cancellation.is_cancelled() {
            return self.execute(Decision::default());
        }
        let decision = self.decision(Event::Enable)?;
        self.execute(decision)
    }
    /// 返回关闭确认前，必须已经完成真实store提交门及listener.stop/join。
    pub fn disable(&mut self) -> Result<BackgroundStep, BackgroundError> {
        self.cancellation.cancel();
        let decision = self.decision(Event::Disable)?;
        self.execute(decision)
    }
    pub fn retry_cleanup(&mut self) -> Result<BackgroundStep, BackgroundError> {
        if !matches!(self.status()?, Status::CleanupFailed { .. }) {
            return Err(BackgroundError::InvalidState);
        }
        let decision = self.decision(Event::RetryCleanup)?;
        self.execute(decision)
    }
    /// 控制器毒化后仍可明确重试资源收尾；成功不表示存储回滚，也不会创建新控制器或重启。
    pub fn cleanup_owned(&mut self) -> Result<(), BackgroundError> {
        self.cancellation.cancel();
        if let Some((_, listener)) = self.active.as_mut() {
            listener
                .stop()
                .map_err(|_| BackgroundError::CleanupFailed)?;
            self.active.take();
        }
        Ok(())
    }
    /// 不读取系统剪贴板；只取监听器已经交付的一项。不存在自动定时调用。
    pub fn poll_once(&mut self) -> Result<BackgroundStep, BackgroundError> {
        if self.cancellation.is_cancelled() {
            return self.disable();
        }
        let Some((key, listener)) = self.active.as_mut() else {
            return self.execute(Decision::default());
        };
        let key = *key;
        let value = match listener.try_next() {
            Ok(ListenerPoll::Empty) => return self.execute(Decision::default()),
            Ok(ListenerPoll::Text(value)) => value,
            Ok(ListenerPoll::Closed) | Err(_) => {
                let mut step = self.disable()?;
                step.reports.push(Failure::UnexpectedStop);
                return Ok(step);
            }
        };
        let mut decision = self.decision(Event::Observed(Observation {
            key,
            sequence: value.sequence,
            text: value.text,
        }))?;
        let request = decision.append.take();
        let mut step = self.execute(decision)?;
        if let Some(request) = request {
            let cancellation = self.cancellation.clone();
            let control = &self.control;
            let store = &mut self.store;
            let committed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                control.commit(&request, |text| {
                    if cancellation.is_cancelled() {
                        return Err(StoreFailure::Cancelled);
                    }
                    store.append(text, &cancellation)
                })
            }));
            match committed {
                Ok(Ok(outcome)) => step.commit = Some(outcome),
                Ok(Err(ControlError::Poisoned)) | Err(_) => {
                    // 存储可能已经发生，不能称作回滚或重试。即使控制器被毒化也独立停自己的资源。
                    let _ = self.cleanup_owned();
                    return Err(BackgroundError::ControlPoisoned);
                }
            }
        }
        Ok(step)
    }
}
impl<F: ListenerFactory, S: BackgroundStore> Drop for HistoryBackground<F, S> {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Some((_, listener)) = self.active.as_mut() {
            let _ = listener.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    #[derive(Default)]
    struct Audit {
        starts: usize,
        stops: usize,
        polls: usize,
        writes: usize,
        saved: Vec<String>,
        stop_failures: usize,
        start_fail: bool,
        cancel_start: bool,
        panic_store: bool,
        store_error: Option<StoreFailure>,
        values: VecDeque<ListenerPoll>,
    }
    struct Factory(Arc<Mutex<Audit>>);
    struct Listener(Arc<Mutex<Audit>>);
    struct Store(Arc<Mutex<Audit>>);
    impl ListenerFactory for Factory {
        fn start(
            &mut self,
            initial: InitialCapturePolicy,
            cancel: &Cancellation,
        ) -> Result<Box<dyn BackgroundListener>, BackgroundError> {
            assert_eq!(initial, InitialCapturePolicy::FutureNotificationsOnly);
            let mut audit = self.0.lock().unwrap();
            audit.starts += 1;
            if audit.cancel_start {
                cancel.cancel();
            }
            if audit.start_fail {
                return Err(BackgroundError::ListenerFailed);
            }
            Ok(Box::new(Listener(self.0.clone())))
        }
    }
    impl BackgroundListener for Listener {
        fn session(&self) -> u64 {
            7
        }
        fn baseline_sequence(&self) -> u32 {
            10
        }
        fn try_next(&mut self) -> Result<ListenerPoll, BackgroundError> {
            let mut audit = self.0.lock().unwrap();
            audit.polls += 1;
            Ok(audit.values.pop_front().unwrap_or(ListenerPoll::Empty))
        }
        fn stop(&mut self) -> Result<(), BackgroundError> {
            let mut audit = self.0.lock().unwrap();
            audit.stops += 1;
            if audit.stop_failures > 0 {
                audit.stop_failures -= 1;
                return Err(BackgroundError::CleanupFailed);
            }
            Ok(())
        }
    }
    impl BackgroundStore for Store {
        fn append(&mut self, text: &str, cancel: &Cancellation) -> Result<(), StoreFailure> {
            assert!(!cancel.is_cancelled());
            let mut audit = self.0.lock().unwrap();
            audit.writes += 1;
            if audit.panic_store {
                drop(audit);
                panic!("合成存储异常");
            }
            if let Some(error) = audit.store_error {
                return Err(error);
            }
            audit.saved.push(text.into());
            Ok(())
        }
    }
    fn driver() -> (HistoryBackground<Factory, Store>, Arc<Mutex<Audit>>) {
        let audit = Arc::new(Mutex::new(Audit::default()));
        (
            HistoryBackground::new(Factory(audit.clone()), Store(audit.clone())),
            audit,
        )
    }
    fn text(sequence: u32, body: &str) -> ListenerPoll {
        ListenerPoll::Text(ObservedText {
            sequence,
            text: body.into(),
        })
    }

    #[test]
    fn default_and_poll_are_inert_without_explicit_enable() {
        let (mut driver, audit) = driver();
        assert_eq!(driver.status().unwrap(), Status::Stopped);
        driver.poll_once().unwrap();
        assert_eq!(
            (audit.lock().unwrap().starts, driver.active.is_none()),
            (0, true)
        );
        assert_eq!(audit.lock().unwrap().writes, 0);
    }
    #[test]
    fn enable_only_establishes_future_notifications_and_poll_is_bounded_to_one_item() {
        let (mut driver, audit) = driver();
        audit.lock().unwrap().values = VecDeque::from([text(11, " 文😀 "), text(12, "")]);
        driver.enable(Cancellation::default()).unwrap();
        assert_eq!(audit.lock().unwrap().writes, 0);
        assert_eq!(
            driver.poll_once().unwrap().commit,
            Some(CommitDisposition::Stored)
        );
        assert_eq!(audit.lock().unwrap().saved, [" 文😀 "]);
        assert_eq!(
            driver.poll_once().unwrap().commit,
            Some(CommitDisposition::Stored)
        );
        assert_eq!(audit.lock().unwrap().saved, [" 文😀 ", ""]);
    }
    #[test]
    fn baseline_and_duplicate_records_never_reach_store() {
        let (mut driver, audit) = driver();
        audit.lock().unwrap().values = VecDeque::from([
            text(10, "synthetic old"),
            text(11, "synthetic"),
            text(11, "synthetic duplicate"),
        ]);
        driver.enable(Cancellation::default()).unwrap();
        assert_eq!(
            driver.poll_once().unwrap().dropped,
            Some(DropReason::DuplicateSequence)
        );
        driver.poll_once().unwrap();
        assert_eq!(
            driver.poll_once().unwrap().dropped,
            Some(DropReason::DuplicateSequence)
        );
        assert_eq!(audit.lock().unwrap().writes, 1);
    }
    #[test]
    fn disable_then_poll_does_not_deliver_old_receiver_data_to_store() {
        let (mut driver, audit) = driver();
        audit
            .lock()
            .unwrap()
            .values
            .push_back(text(11, "synthetic late"));
        driver.enable(Cancellation::default()).unwrap();
        assert_eq!(driver.disable().unwrap().status, Status::Stopped);
        driver.poll_once().unwrap();
        assert_eq!(
            (audit.lock().unwrap().writes, driver.active.is_none()),
            (0, true)
        );
    }
    #[test]
    fn cancellation_before_or_during_start_never_polls_or_appends() {
        for during in [false, true] {
            let (mut driver, audit) = driver();
            let cancel = Cancellation::default();
            if during {
                audit.lock().unwrap().cancel_start = true;
            } else {
                cancel.cancel();
            }
            assert_eq!(driver.enable(cancel).unwrap().status, Status::Stopped);
            assert_eq!(audit.lock().unwrap().writes, 0);
            assert_eq!(audit.lock().unwrap().polls, 0);
        }
    }
    #[test]
    fn start_failure_is_reported_and_has_no_background_retry() {
        let (mut driver, audit) = driver();
        audit.lock().unwrap().start_fail = true;
        assert_eq!(
            driver.enable(Cancellation::default()).unwrap().reports,
            [Failure::StartFailed]
        );
        driver.poll_once().unwrap();
        driver.poll_once().unwrap();
        assert_eq!(audit.lock().unwrap().starts, 1);
    }
    #[test]
    fn cleanup_failure_keeps_resource_slot_until_explicit_retry() {
        let (mut driver, audit) = driver();
        driver.enable(Cancellation::default()).unwrap();
        audit.lock().unwrap().stop_failures = 1;
        assert!(matches!(
            driver.disable().unwrap().status,
            Status::CleanupFailed { .. }
        ));
        assert!(matches!(
            driver.enable(Cancellation::default()),
            Err(BackgroundError::InvalidState)
        ));
        assert_eq!(driver.retry_cleanup().unwrap().status, Status::Stopped);
        assert_eq!(audit.lock().unwrap().starts, 1);
    }
    #[test]
    fn unknown_store_result_is_not_retried_or_reported_as_rollback() {
        let (mut driver, audit) = driver();
        audit.lock().unwrap().store_error = Some(StoreFailure::Unknown);
        audit.lock().unwrap().values =
            VecDeque::from([text(11, "synthetic"), text(11, "synthetic")]);
        driver.enable(Cancellation::default()).unwrap();
        assert_eq!(
            driver.poll_once().unwrap().commit,
            Some(CommitDisposition::Failed(StoreFailure::Unknown))
        );
        assert_eq!(
            driver.poll_once().unwrap().dropped,
            Some(DropReason::DuplicateSequence)
        );
        assert_eq!(audit.lock().unwrap().writes, 1);
    }
    #[test]
    fn poisoned_commit_still_stops_owned_listener_without_using_poisoned_controller() {
        let (mut driver, audit) = driver();
        audit.lock().unwrap().panic_store = true;
        audit
            .lock()
            .unwrap()
            .values
            .push_back(text(11, "synthetic"));
        driver.enable(Cancellation::default()).unwrap();
        assert!(matches!(
            driver.poll_once(),
            Err(BackgroundError::ControlPoisoned)
        ));
        assert!(driver.active.is_none());
        assert_eq!(audit.lock().unwrap().stops, 1);
    }
    #[test]
    fn closed_producer_is_cleaned_up_and_never_restarted_automatically() {
        let (mut driver, audit) = driver();
        audit.lock().unwrap().values.push_back(ListenerPoll::Closed);
        driver.enable(Cancellation::default()).unwrap();
        let step = driver.poll_once().unwrap();
        assert_eq!(step.status, Status::Stopped);
        assert_eq!(step.reports, [Failure::UnexpectedStop]);
        driver.poll_once().unwrap();
        assert_eq!(audit.lock().unwrap().starts, 1);
    }
    #[test]
    fn poisoned_controller_does_not_prevent_explicit_resource_cleanup_retry() {
        let (mut driver, audit) = driver();
        {
            let mut state = audit.lock().unwrap();
            state.panic_store = true;
            state.stop_failures = 1;
            state.values.push_back(text(11, "synthetic"));
        }
        driver.enable(Cancellation::default()).unwrap();
        assert!(matches!(
            driver.poll_once(),
            Err(BackgroundError::ControlPoisoned)
        ));
        assert!(driver.active.is_some());
        driver.cleanup_owned().unwrap();
        assert!(driver.active.is_none());
        assert!(matches!(
            driver.status(),
            Err(BackgroundError::ControlPoisoned)
        ));
        assert_eq!(audit.lock().unwrap().starts, 1);
    }
    #[test]
    fn store_error_mapping_preserves_unknown_effect_even_when_code_is_cancelled() {
        let mut error = HistoryError::new("cancelled");
        error.effect_state = "unknown";
        assert_eq!(store_failure(error), StoreFailure::Unknown);
        assert_eq!(
            store_failure(HistoryError::new("history_conflict")),
            StoreFailure::Conflict
        );
        assert_eq!(
            store_failure(HistoryError::new("cancelled")),
            StoreFailure::Cancelled
        );
    }
}
