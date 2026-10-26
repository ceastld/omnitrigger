//! 离散输入的有界交付屏障。worker 只注入并回执，不访问核心状态机。
use super::{replay_input, BackendEvent, Button, InputKind, KeyMetadata};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::{LPARAM, WPARAM},
    UI::{
        Input::KeyboardAndMouse::{SendInput, INPUT},
        WindowsAndMessaging::{GetForegroundWindow, PostThreadMessageW},
    },
};

pub(super) const ACK_MESSAGE: u32 = super::COMMAND_MESSAGE + 1;
const CAPACITY: usize = 512;
const BATCH: usize = 128;
static WORKER_ACTIVE: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
pub(super) struct Tag {
    pub kind: InputKind,
    pub epoch: u64,
    release: Option<INPUT>,
}

struct Entry {
    input: INPUT,
    tag: Tag,
    foreground: usize,
}

enum Queued {
    Input(Entry),
    Action {
        event: BackendEvent,
        foreground: usize,
    },
}

struct Job {
    sequence: u64,
    inputs: Vec<INPUT>,
    tags: Vec<Tag>,
    foreground: usize,
    cleanup: bool,
}

enum Work {
    Submit(Job),
    Stop,
}

struct Ack {
    sequence: u64,
    sent: usize,
    elapsed: Duration,
    skipped: bool,
}

struct Flight {
    sequence: u64,
    tags: Vec<Tag>,
    cleanup: bool,
}

struct StopCleanup {
    delivered: BTreeSet<Button>,
    affected: BTreeSet<Button>,
    templates: BTreeMap<Button, INPUT>,
    pending_sequence: Option<u64>,
}

struct Worker {
    jobs: SyncSender<Work>,
    acknowledgements: Receiver<Ack>,
    stop: Arc<Mutex<Option<StopCleanup>>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Worker {
    fn start(thread_id: u32) -> Result<Self, String> {
        if WORKER_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("上一个输入补发线程尚未收尾，请稍后再重建监听。".into());
        }
        let (jobs, receiver) = mpsc::sync_channel(1);
        let (acknowledgements, acknowledgements_rx) = mpsc::sync_channel(1);
        let stop = Arc::new(Mutex::new(None));
        let worker_stop = stop.clone();
        let result = thread::Builder::new()
            .name("omnitrigger-replay".into())
            .spawn(move || {
                struct ActiveGuard;
                impl Drop for ActiveGuard {
                    fn drop(&mut self) {
                        WORKER_ACTIVE.store(false, Ordering::Release);
                    }
                }
                let _guard = ActiveGuard;
                let mut completed: Option<(u64, Vec<Tag>, usize)> = None;
                while let Ok(work) = receiver.recv() {
                    if finish_stop(&worker_stop, completed.as_ref()) {
                        return;
                    }
                    let Work::Submit(job) = work else {
                        return;
                    };
                    let skipped = !job.cleanup
                        && unsafe { GetForegroundWindow() }.0 as usize != job.foreground;
                    let started = Instant::now();
                    let sent = if skipped {
                        0
                    } else {
                        (unsafe { SendInput(&job.inputs, std::mem::size_of::<INPUT>() as i32) })
                            as usize
                    };
                    let ack = Ack {
                        sequence: job.sequence,
                        sent,
                        elapsed: started.elapsed(),
                        skipped,
                    };
                    completed = Some((job.sequence, job.tags, sent));
                    let _ = acknowledgements.try_send(ack);
                    let _ =
                        unsafe { PostThreadMessageW(thread_id, ACK_MESSAGE, WPARAM(0), LPARAM(0)) };
                    if finish_stop(&worker_stop, completed.as_ref()) {
                        return;
                    }
                }
                finish_stop(&worker_stop, completed.as_ref());
            });
        let handle = match result {
            Ok(handle) => handle,
            Err(error) => {
                WORKER_ACTIVE.store(false, Ordering::Release);
                return Err(format!("无法启动输入补发线程：{error}"));
            }
        };
        Ok(Self {
            jobs,
            acknowledgements: acknowledgements_rx,
            stop,
            handle: Some(handle),
        })
    }

    fn shutdown(&mut self, cleanup: StopCleanup, idle: bool) {
        if let Ok(mut stop) = self.stop.lock() {
            *stop = Some(cleanup);
        }
        // 若任务尚在槽中，worker 取出它后会先检查 stop，不再执行普通输入。
        let _ = self.jobs.try_send(Work::Stop);
        if idle {
            // 无待交付输入与释放义务时，worker 不会进入原生注入，正常重建无需碰运气。
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }
}

fn finish_stop(
    stop: &Mutex<Option<StopCleanup>>,
    completed: Option<&(u64, Vec<Tag>, usize)>,
) -> bool {
    let Some(mut cleanup) = stop.lock().ok().and_then(|mut stop| stop.take()) else {
        return false;
    };
    if let Some((sequence, tags, sent)) = completed {
        if cleanup.pending_sequence == Some(*sequence) {
            for tag in tags.iter().take(*sent) {
                apply_edge_set(&mut cleanup.delivered, tag.kind);
                if let InputKind::Down(button) = tag.kind {
                    if let Some(release) = tag.release {
                        cleanup.templates.insert(button, release);
                    }
                }
            }
        }
    }
    let inputs: Vec<_> = cleanup
        .affected
        .intersection(&cleanup.delivered)
        .filter_map(|button| cleanup.templates.get(button).copied())
        .collect();
    if !inputs.is_empty() {
        // 原生调用无法取消；监听关闭后的最后释放属于尽力收尾，不声称无损。
        let _ = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    }
    true
}

fn apply_edge_set(delivered: &mut BTreeSet<Button>, kind: InputKind) {
    match kind {
        InputKind::Down(button) => {
            delivered.insert(button);
        }
        InputKind::Up(button) => {
            delivered.remove(&button);
        }
        InputKind::Wheel { .. } => {}
    }
}

pub(super) enum Effect {
    Action(BackendEvent),
    Delivered(Vec<Tag>),
    Timing(Duration),
    Failed {
        affected: BTreeSet<Button>,
        message: String,
    },
}

pub(super) struct ReplayRuntime {
    worker: Option<Worker>,
    queue: VecDeque<Queued>,
    flight: Option<Flight>,
    epochs: BTreeMap<Button, u64>,
    next_epoch: u64,
    delivered: BTreeMap<Button, u64>,
    templates: BTreeMap<Button, INPUT>,
    recovery: BTreeSet<Button>,
    fatal_cleanup: bool,
    next_sequence: u64,
}

impl ReplayRuntime {
    pub fn new() -> Self {
        Self {
            worker: None,
            queue: VecDeque::new(),
            flight: None,
            epochs: BTreeMap::new(),
            next_epoch: 0,
            delivered: BTreeMap::new(),
            templates: BTreeMap::new(),
            recovery: BTreeSet::new(),
            fatal_cleanup: false,
            next_sequence: 0,
        }
    }

    pub fn start(&mut self, thread_id: u32) -> Result<(), String> {
        self.worker = Some(Worker::start(thread_id)?);
        Ok(())
    }

    pub fn new_press(&mut self, button: Button) {
        self.next_epoch = self.next_epoch.wrapping_add(1);
        self.epochs.insert(button, self.next_epoch);
    }

    pub fn epoch(&self, button: Button) -> u64 {
        self.epochs.get(&button).copied().unwrap_or(0)
    }
    pub fn busy(&self) -> bool {
        self.flight.is_some() || !self.queue.is_empty() || !self.recovery.is_empty()
    }
    pub fn retry_cleanup(&mut self) {
        self.fatal_cleanup = false;
    }
    pub fn is_fatal(&self) -> bool {
        self.fatal_cleanup
    }
    pub fn can_pass_release(&self, button: Button) -> bool {
        self.fatal_cleanup && self.delivered.contains_key(&button)
    }

    pub fn passed(&mut self, kind: InputKind, metadata: &BTreeMap<Button, KeyMetadata>) {
        match kind {
            InputKind::Down(button) => {
                if self.fatal_cleanup {
                    self.recovery.remove(&button);
                }
                self.delivered.insert(button, self.epoch(button));
                self.templates
                    .insert(button, replay_input(InputKind::Up(button), metadata));
            }
            InputKind::Up(button) => {
                self.delivered.remove(&button);
                self.recovery.remove(&button);
            }
            InputKind::Wheel { .. } => {}
        }
    }

    pub fn defer_action(
        &mut self,
        event: BackendEvent,
        foreground: usize,
    ) -> Result<Option<BackendEvent>, ()> {
        if !self.busy() {
            return Ok(Some(event));
        }
        if self.size() == CAPACITY {
            return Err(());
        }
        self.queue.push_back(Queued::Action { event, foreground });
        Ok(None)
    }

    fn size(&self) -> usize {
        self.queue.len() + self.flight.as_ref().map_or(0, |flight| flight.tags.len())
    }

    pub fn enqueue(
        &mut self,
        inputs: Vec<INPUT>,
        kinds: Vec<InputKind>,
        foreground: usize,
        metadata: &BTreeMap<Button, KeyMetadata>,
    ) -> Result<(), BTreeSet<Button>> {
        if self.size().saturating_add(kinds.len()) > CAPACITY {
            return Err(self.abandon(&kinds));
        }
        for (input, kind) in inputs.into_iter().zip(kinds) {
            let epoch = match kind {
                InputKind::Down(button) | InputKind::Up(button) => self.epoch(button),
                InputKind::Wheel { .. } => 0,
            };
            let release = match kind {
                InputKind::Down(button) | InputKind::Up(button) => {
                    Some(replay_input(InputKind::Up(button), metadata))
                }
                InputKind::Wheel { .. } => None,
            };
            self.queue.push_back(Queued::Input(Entry {
                input,
                tag: Tag {
                    kind,
                    epoch,
                    release,
                },
                foreground,
            }));
        }
        Ok(())
    }

    pub fn abandon(&mut self, extra: &[InputKind]) -> BTreeSet<Button> {
        let mut affected = self.recovery.clone();
        for kind in extra
            .iter()
            .copied()
            .chain(self.queue.iter().filter_map(|queued| match queued {
                Queued::Input(entry) => Some(entry.tag.kind),
                _ => None,
            }))
            .chain(
                self.flight
                    .iter()
                    .flat_map(|flight| flight.tags.iter().map(|tag| tag.kind)),
            )
        {
            if let InputKind::Down(button) | InputKind::Up(button) = kind {
                affected.insert(button);
            }
        }
        self.queue.clear();
        self.recovery = affected.clone();
        affected
    }

    pub fn pump(&mut self, foreground: usize) -> Vec<Effect> {
        let mut effects = Vec::new();
        if self.fatal_cleanup && self.flight.is_none() {
            return effects;
        }
        let acknowledgement = self
            .worker
            .as_ref()
            .map(|worker| worker.acknowledgements.try_recv());
        if matches!(acknowledgement, Some(Err(mpsc::TryRecvError::Disconnected))) {
            let kinds: Vec<_> = self
                .flight
                .take()
                .into_iter()
                .flat_map(|flight| flight.tags)
                .map(|tag| tag.kind)
                .collect();
            let affected = self.abandon(&kinds);
            self.fatal_cleanup = true;
            effects.push(Effect::Failed {
                affected,
                message: "输入补发线程已断开，监听已暂停；请松开按键后重建监听。".into(),
            });
            return effects;
        }
        if let Some(Ok(ack)) = acknowledgement {
            if let Some(flight) = self.flight.take() {
                if ack.sequence != flight.sequence {
                    self.flight = Some(flight);
                    return effects;
                }
                effects.push(Effect::Timing(ack.elapsed));
                for tag in flight.tags.iter().take(ack.sent) {
                    match tag.kind {
                        InputKind::Down(button) => {
                            self.delivered.insert(button, tag.epoch);
                            if let Some(release) = tag.release {
                                self.templates.insert(button, release);
                            }
                        }
                        InputKind::Up(button) => {
                            if self.delivered.get(&button) == Some(&tag.epoch) {
                                self.delivered.remove(&button);
                            }
                        }
                        InputKind::Wheel { .. } => {}
                    }
                }
                if ack.sent == flight.tags.len() && !ack.skipped {
                    effects.push(Effect::Delivered(flight.tags));
                    self.recovery
                        .retain(|button| self.delivered.contains_key(button));
                } else {
                    let kinds: Vec<_> = flight.tags.iter().map(|tag| tag.kind).collect();
                    let affected = self.abandon(&kinds);
                    if flight.cleanup {
                        self.fatal_cleanup = true;
                    }
                    effects.push(Effect::Failed {
                        affected,
                        message: if ack.skipped {
                            "补发前前台窗口已改变，旧输入已取消，监听已暂停。".into()
                        } else {
                            format!(
                                "输入补发失败（{}/{}），监听已暂停；释放收尾将尽力完成。",
                                ack.sent,
                                flight.tags.len()
                            )
                        },
                    });
                }
            }
        }
        if self.flight.is_some() || self.fatal_cleanup {
            return effects;
        }
        self.recovery
            .retain(|button| self.delivered.contains_key(button));
        let cleanup = !self.recovery.is_empty();
        let mut inputs = Vec::new();
        let mut tags = Vec::new();
        if cleanup {
            for button in &self.recovery {
                if let Some(input) = self.templates.get(button) {
                    inputs.push(*input);
                    tags.push(Tag {
                        kind: InputKind::Up(*button),
                        epoch: self.delivered[button],
                        release: Some(*input),
                    });
                }
            }
        } else {
            for _ in 0..64 {
                if let Some(Queued::Action { .. }) = self.queue.front() {
                    if let Some(Queued::Action {
                        event,
                        foreground: target,
                    }) = self.queue.pop_front()
                    {
                        if target == foreground {
                            effects.push(Effect::Action(event));
                        }
                    }
                } else {
                    break;
                }
            }
            if effects
                .iter()
                .any(|effect| matches!(effect, Effect::Action(_)))
            {
                return effects;
            }
            while tags.len() < BATCH {
                let Some(Queued::Input(entry)) = self.queue.front() else {
                    break;
                };
                if entry.foreground != foreground {
                    let kinds: Vec<_> = tags.iter().map(|tag: &Tag| tag.kind).collect();
                    let affected = self.abandon(&kinds);
                    effects.push(Effect::Failed {
                        affected,
                        message: "补发前前台窗口已改变，旧输入已取消，监听已暂停。".into(),
                    });
                    return effects;
                }
                let Some(Queued::Input(entry)) = self.queue.pop_front() else {
                    unreachable!()
                };
                inputs.push(entry.input);
                tags.push(entry.tag);
            }
        }
        if inputs.is_empty() {
            return effects;
        }
        self.next_sequence = self.next_sequence.wrapping_add(1);
        let sequence = self.next_sequence;
        let job = Job {
            sequence,
            inputs,
            tags: tags.clone(),
            foreground,
            cleanup,
        };
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.jobs.try_send(Work::Submit(job)).is_ok())
        {
            self.flight = Some(Flight {
                sequence,
                tags,
                cleanup,
            });
        } else {
            let kinds: Vec<_> = tags.iter().map(|tag| tag.kind).collect();
            let affected = self.abandon(&kinds);
            self.fatal_cleanup = true;
            effects.push(Effect::Failed {
                affected,
                message: "输入补发线程不可用，监听已暂停。".into(),
            });
        }
        effects
    }
}

impl Drop for ReplayRuntime {
    fn drop(&mut self) {
        let idle = !self.busy();
        let affected = self.abandon(&[]);
        if let Some(worker) = &mut self.worker {
            worker.shutdown(
                StopCleanup {
                    delivered: self.delivered.keys().copied().collect(),
                    affected,
                    templates: std::mem::take(&mut self.templates),
                    pending_sequence: self.flight.as_ref().map(|flight| flight.sequence),
                },
                idle,
            );
        }
    }
}

#[cfg(test)]
#[path = "replay_runtime_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "replay_platform_tests.rs"]
mod platform_tests;
