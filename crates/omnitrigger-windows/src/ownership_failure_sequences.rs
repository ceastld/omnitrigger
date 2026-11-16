//! 直接作为 replay_owned_left.rs 的测试子模块引入；只使用既有请求/worker/ACK/epoch类型。
//! 所有worker均为手工通道且handle=None，不启动线程，不调用permits的Down分支或SendInput。
//! 固定断连、取消、交付未知和外部按压周期交错时的所有权约束。
use super::*;

const FOREGROUND: usize = 0x55;
type ReplyRx = Receiver<Result<(), OwnedLeftInputError>>;

fn request(owner: u64, down: bool) -> (OwnedLeftRequest, ReplyRx) {
    let (mut request, rx) =
        OwnedLeftRequest::fixture(owner, down, Arc::new(OwnedLeftGate::default()));
    request.deadline = Instant::now() + Duration::from_secs(60);
    (request, rx)
}

fn fake_runtime() -> (ReplayRuntime, Receiver<Work>, SyncSender<Ack>) {
    let (jobs, jobs_rx) = mpsc::sync_channel(1);
    let (acks, acks_rx) = mpsc::sync_channel(1);
    let mut runtime = ReplayRuntime::new();
    runtime.worker = Some(Worker {
        jobs,
        acknowledgements: acks_rx,
        stop: Arc::new(Mutex::new(None)),
        handle: None,
    });
    (runtime, jobs_rx, acks)
}

fn job(jobs: &Receiver<Work>) -> Job {
    match jobs.try_recv().expect("应有一个既有worker job") {
        Work::Submit(job) => job,
        Work::Stop => panic!("不能提前停止"),
    }
}

fn ack(runtime: &mut ReplayRuntime, acks: &SyncSender<Ack>, job: &Job, sent: usize) {
    assert!(acks
        .try_send(Ack {
            sequence: job.sequence,
            sent,
            elapsed: Duration::ZERO,
            skipped: false,
        })
        .is_ok());
    runtime.pump(FOREGROUND);
}

fn confirmed_down(
    runtime: &mut ReplayRuntime,
    jobs: &Receiver<Work>,
    acks: &SyncSender<Ack>,
    owner: u64,
) -> u64 {
    let (down, rx) = request(owner, true);
    runtime.owned_left_request(down, false);
    runtime.pump(FOREGROUND);
    let down_job = job(jobs);
    let epoch = down_job.tags[0].epoch;
    ack(runtime, acks, &down_job, 1);
    assert!(rx.try_recv().unwrap().is_ok());
    assert_eq!(runtime.delivered.get(&left()), Some(&epoch));
    epoch
}

#[test]
fn pending_cancel_must_terminally_reply_to_the_same_owner_release_waiter() {
    let mut runtime = ReplayRuntime::new();
    let (down, down_rx) = request(101, true);
    runtime.owned_left_request(down, false);
    let (up, up_rx) = request(101, false);
    runtime.owned_left_request(up, false);
    assert_eq!(runtime.owned_left.waiters.len(), 1);
    // 原有Up路径已把同一gate.cancelled置true；Down尚未提交worker。
    assert!(!runtime.pump_owned_left());
    assert_eq!(down_rx.try_recv().unwrap().unwrap_err().code, "cancelled");
    assert!(!runtime.owned_left_active());
    assert!(runtime.delivered.is_empty());
    // 固定快照已补齐此分支；保留回归，避免waiters及生产Slot再次滞留。
    assert!(up_rx
        .try_recv()
        .expect("无Down交付时应立即终结Up等待者")
        .is_ok());
    assert!(runtime.owned_left.waiters.is_empty());
}

#[test]
fn queued_cancel_or_expiry_before_admission_never_creates_a_press() {
    for expired in [false, true] {
        let mut runtime = ReplayRuntime::new();
        let (mut down, rx) = request(102, true);
        if expired {
            down.deadline = Instant::now() - Duration::from_millis(1);
        } else {
            down.gate.cancelled.store(true, Ordering::Release);
        }
        runtime.owned_left_request(down, false);
        assert!(rx.try_recv().unwrap().is_err());
        assert!(!runtime.owned_left_active());
        assert!(!runtime.owned_left_pending());
        assert!(runtime.delivered.is_empty());
    }
}

#[test]
fn worker_exit_before_submit_must_close_the_pending_request_without_a_debt() {
    let (mut runtime, jobs, acks) = fake_runtime();
    let (down, rx) = request(110, true);
    runtime.owned_left_request(down, false);
    assert!(runtime.flight.is_none());
    drop(acks);
    let effects = runtime.pump(FOREGROUND);
    assert!(effects.iter().any(|e| matches!(e, Effect::Failed { .. })));
    assert!(matches!(jobs.try_recv(), Err(mpsc::TryRecvError::Empty)));
    assert!(runtime.delivered.is_empty());
    assert!(
        !runtime.owned_left_pending(),
        "从未提交worker的请求也必须由断连分支终结"
    );
    assert!(!runtime.owned_left_active());
    assert!(rx.try_recv().expect("pending请求应收到失败终态").is_err());
}

#[test]
fn ack_channel_exit_must_not_leave_an_owned_in_flight_bit_without_a_flight() {
    let (mut runtime, jobs, acks) = fake_runtime();
    let (down, rx) = request(103, true);
    runtime.owned_left_request(down, false);
    runtime.pump(FOREGROUND);
    let _submitted = job(&jobs);
    drop(acks); // 不知道worker是否实际交付；不能冒称not_started或Succeeded。
    let effects = runtime.pump(FOREGROUND);
    assert!(effects.iter().any(|e| matches!(e, Effect::Failed { .. })));
    assert!(runtime.is_fatal());
    assert!(runtime.flight.is_none());
    assert!(!matches!(rx.try_recv(), Ok(Ok(()))));
    // 审阅快照当前缺口：通用断连分支take了flight，却没有撤销此位。
    assert!(
        !runtime.owned_left.in_flight,
        "未决交付必须转为明确失败债务，不能仍假装有在途job"
    );
}

#[test]
fn ack_channel_exit_during_up_preserves_known_debt_but_clears_phantom_flight() {
    let (mut runtime, jobs, acks) = fake_runtime();
    let epoch = confirmed_down(&mut runtime, &jobs, &acks, 104);
    let (up, rx) = request(104, false);
    runtime.owned_left_request(up, false);
    runtime.pump(FOREGROUND);
    let _up_job = job(&jobs);
    drop(acks);
    runtime.pump(FOREGROUND);
    assert!(runtime.is_fatal());
    assert_eq!(runtime.delivered.get(&left()), Some(&epoch));
    assert!(!matches!(rx.try_recv(), Ok(Ok(()))));
    assert!(runtime.flight.is_none());
    assert!(!runtime.owned_left.in_flight);
    assert!(
        runtime.owned_left_active(),
        "确认过的Down不能因Up回执丢失而清债"
    );
}

#[test]
fn an_ack_without_a_wakeup_is_consumed_by_the_next_existing_pump() {
    let (mut runtime, jobs, acks) = fake_runtime();
    let epoch = confirmed_down(&mut runtime, &jobs, &acks, 105);
    // confirmed_down只把ACK放进既有通道后调用pump；没有PostThreadMessage。
    assert_eq!(runtime.delivered.get(&left()), Some(&epoch));
    assert!(runtime.flight.is_none());
}

#[test]
fn missing_ack_is_not_a_retry_and_drop_transfers_the_exact_pending_sequence() {
    let (mut runtime, jobs, _acks) = fake_runtime();
    let stop = runtime.worker.as_ref().unwrap().stop.clone();
    let (down, rx) = request(106, true);
    runtime.owned_left_request(down, false);
    runtime.pump(FOREGROUND);
    let submitted = job(&jobs);
    assert!(submitted.tags[0].release.is_some());
    for _ in 0..3 {
        runtime.pump(FOREGROUND);
        assert!(matches!(jobs.try_recv(), Err(mpsc::TryRecvError::Empty)));
    }
    runtime.cancel_owned_left();
    drop(runtime);
    let cleanup = stop.lock().unwrap().take().expect("既有worker收尾必须接管");
    assert_eq!(cleanup.pending_sequence, Some(submitted.sequence));
    assert!(cleanup.affected.contains(&left()));
    assert!(
        !cleanup.delivered.contains(&left()),
        "ACK未到，不能虚构已交付"
    );
    assert!(!matches!(rx.try_recv(), Ok(Ok(()))));
    // 不调用finish_stop：它含真实SendInput。源码中completed按同sequence补账再中性Up。
}

#[test]
fn a_lost_down_reply_still_has_the_same_action_owner_for_neutral_release() {
    let (mut runtime, jobs, acks) = fake_runtime();
    let (down, rx) = request(107, true);
    runtime.owned_left_request(down, false);
    runtime.pump(FOREGROUND);
    let submitted = job(&jobs);
    drop(rx); // 调用方未收到成功回执；Action层此前已登记may_be_down。
    ack(&mut runtime, &acks, &submitted, 1);
    assert!(runtime.owned_left_active());
    let epoch = runtime.delivered[&left()];
    let (up, up_rx) = request(107, false);
    runtime.owned_left_request(up, false);
    runtime.pump(FOREGROUND);
    let cleanup = job(&jobs);
    assert!(cleanup.cleanup);
    assert_eq!(cleanup.tags[0].epoch, epoch);
    ack(&mut runtime, &acks, &cleanup, 1);
    assert!(up_rx.try_recv().unwrap().is_ok());
    assert!(!runtime.owned_left_active());
    assert!(runtime.delivered.is_empty());
}

#[test]
fn a_new_epoch_alone_is_not_proof_that_the_old_api_debt_was_released() {
    let (mut runtime, jobs, acks) = fake_runtime();
    let old = confirmed_down(&mut runtime, &jobs, &acks, 108);
    runtime.new_press(left()); // 尚没有passed外部边沿，不能擅自认定OS所有权已转交。
    let fresh = runtime.epoch(left());
    assert_ne!(old, fresh);
    let (up, rx) = request(108, false);
    runtime.owned_left_request(up, false);
    runtime.pump(FOREGROUND);
    let cleanup = job(&jobs);
    assert_eq!(cleanup.tags[0].epoch, old);
    ack(&mut runtime, &acks, &cleanup, 1);
    assert!(rx.try_recv().unwrap().is_ok());
    assert!(runtime.delivered.is_empty());
    assert_eq!(runtime.epoch(left()), fresh);
}

#[test]
fn passed_external_press_is_not_released_by_old_owner_or_shutdown_cleanup() {
    let (mut runtime, jobs, acks) = fake_runtime();
    let stop = runtime.worker.as_ref().unwrap().stop.clone();
    confirmed_down(&mut runtime, &jobs, &acks, 109);
    runtime.new_press(left());
    runtime.passed(InputKind::Down(left()), &BTreeMap::new());
    let fresh = runtime.delivered[&left()];
    let (up, rx) = request(109, false);
    runtime.owned_left_request(up, false);
    assert!(rx.try_recv().unwrap().is_ok());
    assert_eq!(runtime.delivered[&left()], fresh);
    assert!(matches!(jobs.try_recv(), Err(mpsc::TryRecvError::Empty)));
    drop(runtime);
    let cleanup = stop.lock().unwrap().take().unwrap();
    assert!(cleanup.delivered.contains(&left()));
    assert!(
        !cleanup.affected.contains(&left()),
        "不能为旧owner向新物理按压发Up"
    );
}

#[test]
fn neutral_up_guard_ignores_original_target_cancel_and_expiry_until_revoked() {
    let gate = Arc::new(OwnedLeftGate::default());
    gate.cancelled.store(true, Ordering::Release);
    let guard = OwnedLeftJobGuard {
        gate: gate.clone(),
        down: false,
        target: None,
        deadline: Instant::now() - Duration::from_millis(1),
    };
    assert!(guard.permits()); // Up分支只检查原owner是否已撤销，不做Win32窗口查询。
    gate.revoked.store(true, Ordering::Release);
    assert!(!guard.permits());
}
