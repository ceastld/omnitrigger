//! 退出不变量：纯mpsc假worker，不创建线程，不调用SendInput或Down目标查询。
use super::*;

const FOREGROUND: usize = 0x77;
type ReplyRx = Receiver<Result<(), OwnedLeftInputError>>;

fn request(owner: u64, down: bool) -> (OwnedLeftRequest, ReplyRx) {
    let (mut request, reply) =
        OwnedLeftRequest::fixture(owner, down, Arc::new(OwnedLeftGate::default()));
    request.deadline = Instant::now() + Duration::from_secs(60);
    (request, reply)
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
    match jobs.try_recv().unwrap() {
        Work::Submit(job) => job,
        Work::Stop => panic!("不应提交Stop"),
    }
}

fn confirmed_down(
    runtime: &mut ReplayRuntime,
    jobs: &Receiver<Work>,
    acks: &SyncSender<Ack>,
    owner: u64,
) -> u64 {
    let (down, reply) = request(owner, true);
    runtime.owned_left_request(down, false);
    runtime.pump(FOREGROUND);
    let submitted = job(jobs);
    assert!(acks
        .try_send(Ack {
            sequence: submitted.sequence,
            sent: 1,
            elapsed: Duration::ZERO,
            skipped: false,
        })
        .is_ok());
    runtime.pump(FOREGROUND);
    reply.try_recv().unwrap().unwrap();
    submitted.tags[0].epoch
}

fn unavailable(reply: &ReplyRx) {
    assert_eq!(
        reply
            .try_recv()
            .expect("请求必须有明确终态")
            .unwrap_err()
            .code,
        "input_owner_unavailable"
    );
}

fn closed_requests(runtime: &ReplayRuntime) {
    assert!(runtime.flight.is_none());
    assert!(!runtime.owned_left.in_flight);
    assert!(runtime.owned_left.pending.is_none());
    assert!(runtime.owned_left.waiters.is_empty());
}

#[test]
fn exit_before_submit_replies_to_down_and_drops_all_release_waiters_without_debt() {
    let (mut runtime, jobs, acks) = fake_runtime();
    let (down, down_reply) = request(201, true);
    runtime.owned_left_request(down, false);
    let (up, up_reply) = request(201, false);
    runtime.owned_left_request(up, false);
    drop(acks);
    runtime.pump(FOREGROUND);
    unavailable(&down_reply);
    up_reply.try_recv().unwrap().unwrap();
    closed_requests(&runtime);
    assert!(!runtime.owned_left_active());
    assert!(runtime.delivered.is_empty());
    assert!(runtime.recovery.is_empty());
    assert!(matches!(jobs.try_recv(), Err(mpsc::TryRecvError::Empty)));
}

#[test]
fn unknown_down_exit_fails_waiters_and_future_same_owner_up_without_fabricated_delivery() {
    let (mut runtime, jobs, acks) = fake_runtime();
    let (down, down_reply) = request(202, true);
    let gate = down.gate.clone();
    runtime.owned_left_request(down, false);
    runtime.pump(FOREGROUND);
    let _submitted = job(&jobs);
    let epoch = runtime.owned_left.press.as_ref().unwrap().epoch;
    let (up, up_reply) = request(202, false);
    runtime.owned_left_request(up, false);
    drop(acks);
    runtime.pump(FOREGROUND);
    unavailable(&down_reply);
    unavailable(&up_reply);
    closed_requests(&runtime);
    let press = runtime.owned_left.press.as_ref().unwrap();
    assert_eq!((press.owner, press.epoch), (202, epoch));
    assert!(Arc::ptr_eq(&gate, &press.gate));
    assert!(gate.cancelled.load(Ordering::Acquire));
    assert!(!gate.revoked.load(Ordering::Acquire));
    assert!(
        runtime.delivered.is_empty(),
        "未知Down不能伪造confirmed交付"
    );
    assert!(runtime.recovery.contains(&left()));
    let (up, retry_reply) = request(202, false);
    runtime.owned_left_request(up, false);
    runtime.pump(FOREGROUND);
    unavailable(&retry_reply);
    closed_requests(&runtime);
    assert!(
        runtime.owned_left_active(),
        "缺少delivered不能把未知owner的Up判为成功"
    );
    let (other, other_reply) = request(999, true);
    runtime.owned_left_request(other, false);
    assert_eq!(
        other_reply.try_recv().unwrap().unwrap_err().code,
        "input_busy"
    );
    assert!(matches!(jobs.try_recv(), Err(mpsc::TryRecvError::Empty)));
}

#[test]
fn unknown_up_exit_fails_all_waiters_and_preserves_confirmed_epoch_on_retry() {
    let (mut runtime, jobs, acks) = fake_runtime();
    let epoch = confirmed_down(&mut runtime, &jobs, &acks, 203);
    let (up, up_reply) = request(203, false);
    runtime.owned_left_request(up, false);
    runtime.pump(FOREGROUND);
    let _submitted = job(&jobs);
    let (waiter, waiter_reply) = request(203, false);
    runtime.owned_left_request(waiter, false);
    drop(acks);
    runtime.pump(FOREGROUND);
    unavailable(&up_reply);
    unavailable(&waiter_reply);
    closed_requests(&runtime);
    assert_eq!(runtime.delivered.get(&left()), Some(&epoch));
    assert!(runtime.templates.contains_key(&left()));
    assert!(runtime.recovery.contains(&left()));
    let (retry, retry_reply) = request(203, false);
    runtime.owned_left_request(retry, false);
    runtime.pump(FOREGROUND);
    unavailable(&retry_reply);
    closed_requests(&runtime);
    assert_eq!(runtime.delivered.get(&left()), Some(&epoch));
    assert_eq!(runtime.owned_left.press.as_ref().unwrap().owner, 203);
    assert!(matches!(jobs.try_recv(), Err(mpsc::TryRecvError::Empty)));
}

#[test]
fn worker_exit_with_idle_confirmed_owner_keeps_debt_without_auto_requeue() {
    let (mut runtime, jobs, acks) = fake_runtime();
    let epoch = confirmed_down(&mut runtime, &jobs, &acks, 204);
    drop(acks);
    let effects = runtime.pump(FOREGROUND);
    assert!(effects.iter().any(|effect| {
        matches!(effect, Effect::Failed { affected, .. } if affected.contains(&left()))
    }));
    assert!(runtime.is_fatal());
    closed_requests(&runtime);
    assert!(runtime.owned_left_active());
    assert_eq!(runtime.delivered.get(&left()), Some(&epoch));
    assert!(runtime.recovery.contains(&left()));
    for _ in 0..3 {
        runtime.cancel_owned_left();
        assert!(runtime.pump(FOREGROUND).is_empty());
        closed_requests(&runtime);
    }
    assert!(matches!(jobs.try_recv(), Err(mpsc::TryRecvError::Empty)));
}

#[test]
fn explicit_retry_cannot_clear_unknown_down_through_the_missing_delivered_fast_path() {
    let (mut runtime, jobs, acks) = fake_runtime();
    let (down, down_reply) = request(205, true);
    runtime.owned_left_request(down, false);
    runtime.pump(FOREGROUND);
    let _submitted = job(&jobs);
    drop(acks);
    runtime.pump(FOREGROUND);
    unavailable(&down_reply);
    runtime.retry_cleanup();
    let (up, up_reply) = request(205, false);
    runtime.owned_left_request(up, false);
    // 即使直接进入owned pump，缺少delivered的快捷出口也不能清掉未知债务。
    assert!(!runtime.pump_owned_left());
    unavailable(&up_reply);
    closed_requests(&runtime);
    assert!(runtime.is_fatal());
    assert!(runtime.delivered.is_empty());
    assert!(runtime.recovery.contains(&left()));
    assert_eq!(runtime.owned_left.press.as_ref().unwrap().owner, 205);
    assert!(matches!(jobs.try_recv(), Err(mpsc::TryRecvError::Empty)));
}
