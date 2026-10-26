use super::*;
use omnitrigger_core::Axis;

const FOREGROUND: usize = 0x55;

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

fn enqueue(runtime: &mut ReplayRuntime, kinds: &[InputKind]) {
    let metadata = BTreeMap::new();
    let inputs = kinds
        .iter()
        .map(|kind| replay_input(*kind, &metadata))
        .collect();
    assert!(runtime
        .enqueue(inputs, kinds.to_vec(), FOREGROUND, &metadata)
        .is_ok());
}

fn next_job(jobs: &Receiver<Work>) -> Job {
    match jobs.try_recv() {
        Ok(Work::Submit(job)) => job,
        _ => panic!("应当有一批待交付输入"),
    }
}

fn acknowledge(acks: &SyncSender<Ack>, job: &Job, sent: usize) {
    assert!(acks
        .send(Ack {
            sequence: job.sequence,
            sent,
            elapsed: Duration::from_millis(3),
            skipped: false,
        })
        .is_ok());
}

fn kinds(job: &Job) -> Vec<InputKind> {
    job.tags.iter().map(|tag| tag.kind).collect()
}

#[test]
fn failed_prefix_delivery_still_releases_a_preexisting_key_whose_up_was_queued() {
    let (mut runtime, jobs, acks) = fake_runtime();
    let ctrl = Button::Key(0xa2);
    let caps = Button::Key(0x14);
    runtime.new_press(ctrl);
    runtime.passed(InputKind::Down(ctrl), &BTreeMap::new());
    runtime.new_press(caps);
    enqueue(&mut runtime, &[InputKind::Down(caps)]);
    runtime.pump(FOREGROUND);
    let prefix = next_job(&jobs);

    enqueue(&mut runtime, &[InputKind::Up(ctrl)]);
    acknowledge(&acks, &prefix, 0);
    let effects = runtime.pump(FOREGROUND);
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::Failed { .. })));
    let cleanup = next_job(&jobs);
    assert!(cleanup.cleanup);
    assert_eq!(kinds(&cleanup), vec![InputKind::Up(ctrl)]);

    acknowledge(&acks, &cleanup, cleanup.tags.len());
    runtime.pump(FOREGROUND);
    assert!(!runtime.busy());
    assert!(runtime.delivered.is_empty());
}

#[test]
fn partial_delivery_cleans_only_the_down_edges_that_were_submitted() {
    let (mut runtime, jobs, acks) = fake_runtime();
    let first = Button::Key(0x41);
    let second = Button::Key(0x42);
    runtime.new_press(first);
    runtime.new_press(second);
    enqueue(
        &mut runtime,
        &[InputKind::Down(first), InputKind::Down(second)],
    );
    runtime.pump(FOREGROUND);
    let submitted = next_job(&jobs);

    acknowledge(&acks, &submitted, 1);
    let effects = runtime.pump(FOREGROUND);
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::Failed { .. })));
    let cleanup = next_job(&jobs);
    assert!(cleanup.cleanup);
    assert_eq!(kinds(&cleanup), vec![InputKind::Up(first)]);

    acknowledge(&acks, &cleanup, 1);
    runtime.pump(FOREGROUND);
    assert!(!runtime.busy());
    assert!(runtime.delivered.is_empty());
}

#[test]
fn queue_overflow_preserves_the_current_release_obligation() {
    let (mut runtime, jobs, acks) = fake_runtime();
    let ctrl = Button::Key(0xa2);
    runtime.new_press(ctrl);
    runtime.passed(InputKind::Down(ctrl), &BTreeMap::new());
    let pending = vec![
        InputKind::Wheel {
            axis: Axis::Vertical,
            delta: 1
        };
        CAPACITY
    ];
    enqueue(&mut runtime, &pending);

    let release = InputKind::Up(ctrl);
    let metadata = BTreeMap::new();
    let result = runtime.enqueue(
        vec![replay_input(release, &metadata)],
        vec![release],
        FOREGROUND,
        &metadata,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains(&ctrl));
    assert!(runtime.queue.is_empty());
    runtime.pump(FOREGROUND);
    let cleanup = next_job(&jobs);
    assert!(cleanup.cleanup);
    assert_eq!(kinds(&cleanup), vec![release]);

    acknowledge(&acks, &cleanup, 1);
    runtime.pump(FOREGROUND);
    assert!(!runtime.busy());
}

#[test]
fn disconnected_worker_becomes_fatal_and_allows_an_owed_physical_release() {
    let (mut runtime, jobs, acks) = fake_runtime();
    let ctrl = Button::Key(0xa2);
    runtime.new_press(ctrl);
    runtime.passed(InputKind::Down(ctrl), &BTreeMap::new());
    enqueue(&mut runtime, &[InputKind::Up(ctrl)]);
    runtime.pump(FOREGROUND);
    let _submitted = next_job(&jobs);
    drop(acks);

    let effects = runtime.pump(FOREGROUND);
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::Failed { .. })));
    assert!(runtime.is_fatal());
    assert!(runtime.flight.is_none());
    assert!(runtime.can_pass_release(ctrl));
    runtime.passed(InputKind::Up(ctrl), &BTreeMap::new());
    assert!(!runtime.can_pass_release(ctrl));
    assert!(!runtime.busy());
}
