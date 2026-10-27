//! 系统调节使用固定的音量、亮度工作线程，不占用普通命令执行配额。
//!
//! 每类动作保持 FIFO；相邻且尚未开始的同向动作合并后，只量化一次硬件步长。
//! 两类动作之间不保证完成顺序，慢亮度接口不会阻塞音量或界面。
use crate::system_action::{AdjustmentTarget, SystemAdjustmentAction};
use omnitrigger_windows::AdjustmentResult;
use std::{
    collections::VecDeque,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{mpsc::TryRecvError, Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
};

/// 待执行、正在执行和未读取结果共用此配额；合并不会额外占用配额。
const CAPACITY: usize = 8;
type Point = (i32, i32);
type Runner =
    Box<dyn FnMut(SystemAdjustmentAction, Point) -> Result<AdjustmentResult, String> + Send>;

#[derive(Debug, PartialEq, Eq)]
pub struct SystemExecutionEvent {
    pub id: u64,
    pub result: Result<AdjustmentResult, String>,
}

#[derive(Clone, Copy)]
struct Batch {
    id: u64,
    action: SystemAdjustmentAction,
    point: Point,
}

impl Batch {
    fn merge(&mut self, next: Self) -> bool {
        if self.id != next.id
            || self.action.target != next.action.target
            || self.action.delta_percent.signum() != next.action.delta_percent.signum()
            || (self.action.target == AdjustmentTarget::Brightness && self.point != next.point)
        {
            return false;
        }
        // 同向连续累加超过全范围时，最终 clamp 与累加 100 个百分点相同。
        self.action.delta_percent =
            (self.action.delta_percent + next.action.delta_percent).clamp(-100, 100);
        true
    }
}

struct State {
    pending: [VecDeque<Batch>; 2],
    results: VecDeque<SystemExecutionEvent>,
    /// 允许同一绑定有多批次；每读取一个结果才释放一个 reservation。
    reserved_ids: Vec<u64>,
    alive: [bool; 2],
    stopping: bool,
}

struct Shared {
    state: Mutex<State>,
    ready: [Condvar; 2],
}

pub struct SystemAdjustmentExecutor {
    shared: Arc<Shared>,
    workers: [Option<JoinHandle<()>>; 2],
    startup_errors: [Option<String>; 2],
}

impl Default for SystemAdjustmentExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemAdjustmentExecutor {
    pub fn new() -> Self {
        Self::with_runners(run_native, run_native)
    }

    fn with_runners<V, B>(volume: V, brightness: B) -> Self
    where
        V: FnMut(SystemAdjustmentAction, Point) -> Result<AdjustmentResult, String>
            + Send
            + 'static,
        B: FnMut(SystemAdjustmentAction, Point) -> Result<AdjustmentResult, String>
            + Send
            + 'static,
    {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                pending: std::array::from_fn(|_| VecDeque::with_capacity(CAPACITY)),
                results: VecDeque::with_capacity(CAPACITY),
                reserved_ids: Vec::with_capacity(CAPACITY),
                alive: [true; 2],
                stopping: false,
            }),
            ready: std::array::from_fn(|_| Condvar::new()),
        });
        let runners: [Runner; 2] = [Box::new(volume), Box::new(brightness)];
        let mut workers = [None, None];
        let mut startup_errors = [None, None];
        for (index, runner) in runners.into_iter().enumerate() {
            let worker_shared = shared.clone();
            let name = ["omnitrigger-volume", "omnitrigger-brightness"][index];
            match thread::Builder::new()
                .name(name.into())
                .spawn(move || work(worker_shared, index, runner))
            {
                Ok(worker) => workers[index] = Some(worker),
                Err(error) => {
                    startup_errors[index] = Some(format!("无法启动系统调节线程：{error}"));
                    if let Ok(mut state) = shared.state.lock() {
                        state.alive[index] = false;
                    }
                }
            }
        }
        Self {
            shared,
            workers,
            startup_errors,
        }
    }

    pub fn submit(
        &self,
        id: u64,
        action: SystemAdjustmentAction,
        point: Point,
    ) -> Result<(), String> {
        action.validate()?;
        let index = target_index(action.target);
        if let Some(error) = &self.startup_errors[index] {
            return Err(error.clone());
        }
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| "系统调节队列不可用。".to_string())?;
        if state.stopping || !state.alive[index] {
            return Err("系统调节执行器已关闭。".into());
        }
        let batch = Batch { id, action, point };
        // 只看该类队尾；不得越过反方向、其他绑定或其他显示器的批次。
        if state.pending[index]
            .back_mut()
            .is_some_and(|pending| pending.merge(batch))
        {
            return Ok(());
        }
        if state.reserved_ids.len() == CAPACITY {
            return Err("系统调节队列已满，请稍后重试。".into());
        }
        state.reserved_ids.push(id);
        state.pending[index].push_back(batch);
        drop(state);
        self.shared.ready[index].notify_one();
        Ok(())
    }

    pub fn try_recv(&self) -> Result<SystemExecutionEvent, TryRecvError> {
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| TryRecvError::Disconnected)?;
        if let Some(event) = state.results.pop_front() {
            if let Some(index) = state.reserved_ids.iter().position(|id| *id == event.id) {
                state.reserved_ids.swap_remove(index);
            }
            return Ok(event);
        }
        if state.stopping || !state.alive.iter().any(|alive| *alive) {
            Err(TryRecvError::Disconnected)
        } else {
            Err(TryRecvError::Empty)
        }
    }

    /// 包含尚未读取结果的批次；同一绑定仍可继续提交相邻滚轮步长。
    pub fn is_running(&self, id: u64) -> bool {
        self.shared
            .state
            .lock()
            .is_ok_and(|state| state.reserved_ids.contains(&id))
    }

    fn close(&self) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.stopping = true;
            state.pending.iter_mut().for_each(VecDeque::clear);
            state.results.clear();
            state.reserved_ids.clear();
        }
        self.shared.ready.iter().for_each(Condvar::notify_all);
    }
}

impl Drop for SystemAdjustmentExecutor {
    fn drop(&mut self) {
        self.close();
        for worker in &mut self.workers {
            // 原生 COM/显示器调用可能耗时，界面销毁不等待它，也不强制终止线程。
            if let Some(worker) = worker.take().filter(JoinHandle::is_finished) {
                let _ = worker.join();
            }
        }
    }
}

fn target_index(target: AdjustmentTarget) -> usize {
    match target {
        AdjustmentTarget::Volume => 0,
        AdjustmentTarget::Brightness => 1,
    }
}

fn run_native(action: SystemAdjustmentAction, point: Point) -> Result<AdjustmentResult, String> {
    omnitrigger_windows::adjust_system_at(action.platform(), point)
        .map_err(|error| error.to_string())
}

fn work(shared: Arc<Shared>, index: usize, mut runner: Runner) {
    // 正常结束和未预期的线程退出都让后续提交明确失败。
    struct WorkerExit(Arc<Shared>, usize);
    impl Drop for WorkerExit {
        fn drop(&mut self) {
            if let Ok(mut state) = self.0.state.lock() {
                state.alive[self.1] = false;
            }
        }
    }
    let _exit = WorkerExit(shared.clone(), index);
    loop {
        let batch = {
            let Ok(mut state) = shared.state.lock() else {
                return;
            };
            loop {
                if state.stopping {
                    return;
                }
                if let Some(batch) = state.pending[index].pop_front() {
                    // 离开队列就是开始边界；此后新请求不能合并到这次原生调用。
                    break batch;
                }
                state = match shared.ready[index].wait(state) {
                    Ok(state) => state,
                    Err(_) => return,
                };
            }
        };
        let result = catch_unwind(AssertUnwindSafe(|| runner(batch.action, batch.point)))
            .unwrap_or_else(|_| Err("系统调节工作线程执行异常。".into()));
        let Ok(mut state) = shared.state.lock() else {
            return;
        };
        if state.stopping {
            return;
        }
        state.results.push_back(SystemExecutionEvent {
            id: batch.id,
            result,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnitrigger_windows::AdjustmentKind;
    use std::{
        sync::mpsc::{self, Receiver, Sender},
        time::{Duration, Instant},
    };

    const TIMEOUT: Duration = Duration::from_secs(3);

    fn action(target: AdjustmentTarget, delta_percent: i16) -> SystemAdjustmentAction {
        SystemAdjustmentAction {
            target,
            delta_percent,
        }
    }

    fn result(action: SystemAdjustmentAction, previous: u8) -> AdjustmentResult {
        let next = (i16::from(previous) + action.delta_percent).clamp(0, 100) as u8;
        AdjustmentResult {
            kind: match action.target {
                AdjustmentTarget::Volume => AdjustmentKind::Volume,
                AdjustmentTarget::Brightness => AdjustmentKind::Brightness,
            },
            previous_percent: previous,
            requested_percent: next,
            current_percent: next,
            target_label: "mock".into(),
            brightness_method: None,
        }
    }

    fn immediate(action: SystemAdjustmentAction, _: Point) -> Result<AdjustmentResult, String> {
        Ok(result(action, 50))
    }

    type Call = (SystemAdjustmentAction, Point);
    fn gated() -> (Runner, Receiver<Call>, Sender<()>) {
        let (calls_tx, calls_rx) = mpsc::channel();
        let (permit_tx, permit_rx) = mpsc::channel();
        let mut current = 50;
        let runner = Box::new(move |action, point| {
            calls_tx
                .send((action, point))
                .map_err(|_| "测试已结束".to_string())?;
            permit_rx.recv().map_err(|_| "测试已结束".to_string())?;
            let value = result(action, current);
            current = value.current_percent;
            Ok(value)
        });
        (runner, calls_rx, permit_tx)
    }

    fn receive(executor: &SystemAdjustmentExecutor) -> SystemExecutionEvent {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            match executor.try_recv() {
                Ok(event) => return event,
                Err(TryRecvError::Empty) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(1));
                }
                other => panic!("未收到系统调节结果：{other:?}"),
            }
        }
    }

    fn wait_results(executor: &SystemAdjustmentExecutor, count: usize) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if executor.shared.state.lock().unwrap().results.len() == count {
                return;
            }
            assert!(Instant::now() < deadline, "未生成预期数量的结果");
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn burst_120_merges_pending_volume_and_preserves_clamped_total() {
        let (runner, calls, permits) = gated();
        let executor = SystemAdjustmentExecutor::with_runners(runner, immediate);
        let up = action(AdjustmentTarget::Volume, 1);
        executor.submit(7, up, (0, 0)).unwrap();
        assert_eq!(calls.recv_timeout(TIMEOUT).unwrap(), (up, (0, 0)));
        for x in 1..120 {
            executor.submit(7, up, (x, 0)).unwrap();
        }
        assert_eq!(executor.shared.state.lock().unwrap().reserved_ids.len(), 2);
        permits.send(()).unwrap();
        assert_eq!(
            calls.recv_timeout(TIMEOUT).unwrap().0,
            action(AdjustmentTarget::Volume, 100)
        );
        permits.send(()).unwrap();
        assert_eq!(receive(&executor).result.unwrap().current_percent, 51);
        assert!(executor.is_running(7));
        assert_eq!(receive(&executor).result.unwrap().current_percent, 100);
        assert!(!executor.is_running(7));
        assert!(calls.try_recv().is_err());
    }

    #[test]
    fn brightness_preserves_direction_binding_and_exact_screen_point_order() {
        let (runner, calls, permits) = gated();
        let executor = SystemAdjustmentExecutor::with_runners(immediate, runner);
        let up = action(AdjustmentTarget::Brightness, 2);
        executor.submit(1, up, (1, 1)).unwrap();
        calls.recv_timeout(TIMEOUT).unwrap();
        executor.submit(1, up, (1, 1)).unwrap();
        executor.submit(1, up, (1, 1)).unwrap();
        executor
            .submit(1, action(AdjustmentTarget::Brightness, -3), (1, 1))
            .unwrap();
        executor.submit(1, up, (1, 1)).unwrap();
        executor.submit(1, up, (2, 1)).unwrap();
        executor.submit(2, up, (2, 1)).unwrap();
        let expected = [
            (1, 4, (1, 1)),
            (1, -3, (1, 1)),
            (1, 2, (1, 1)),
            (1, 2, (2, 1)),
            (2, 2, (2, 1)),
        ];
        permits.send(()).unwrap();
        assert_eq!(receive(&executor).id, 1);
        for (id, delta, point) in expected {
            assert_eq!(
                calls.recv_timeout(TIMEOUT).unwrap(),
                (action(AdjustmentTarget::Brightness, delta), point)
            );
            permits.send(()).unwrap();
            assert_eq!(receive(&executor).id, id);
        }
    }

    #[test]
    fn blocked_brightness_does_not_block_volume() {
        let (runner, calls, permits) = gated();
        let executor = SystemAdjustmentExecutor::with_runners(immediate, runner);
        executor
            .submit(1, action(AdjustmentTarget::Brightness, 1), (0, 0))
            .unwrap();
        calls.recv_timeout(TIMEOUT).unwrap();
        executor
            .submit(2, action(AdjustmentTarget::Volume, 3), (0, 0))
            .unwrap();
        assert_eq!(receive(&executor).id, 2);
        assert!(executor.is_running(1));
        permits.send(()).unwrap();
        assert_eq!(receive(&executor).id, 1);
    }

    #[test]
    fn shared_capacity_includes_running_pending_and_unread_results() {
        let (volume, volume_calls, volume_permits) = gated();
        let (brightness, brightness_calls, brightness_permits) = gated();
        let executor = SystemAdjustmentExecutor::with_runners(volume, brightness);
        for id in 0..CAPACITY as u64 {
            let target = if id % 2 == 0 {
                AdjustmentTarget::Volume
            } else {
                AdjustmentTarget::Brightness
            };
            executor.submit(id, action(target, 1), (0, 0)).unwrap();
        }
        volume_calls.recv_timeout(TIMEOUT).unwrap();
        brightness_calls.recv_timeout(TIMEOUT).unwrap();
        let up = action(AdjustmentTarget::Brightness, 1);
        assert!(executor.submit(99, up, (0, 0)).is_err());
        // 满额时仍可合并已有待执行队尾，不会丢掉同向滚轮步长。
        executor.submit(7, up, (0, 0)).unwrap();
        for _ in 0..CAPACITY / 2 {
            volume_permits.send(()).unwrap();
            brightness_permits.send(()).unwrap();
        }
        wait_results(&executor, CAPACITY);
        assert!(executor.submit(7, up, (0, 0)).is_err());
        receive(&executor);
        executor.submit(99, up, (0, 0)).unwrap();
        assert!(executor.submit(100, up, (0, 0)).is_err());
        brightness_permits.send(()).unwrap();
        wait_results(&executor, CAPACITY);
        for _ in 0..CAPACITY {
            receive(&executor);
        }
        assert!(executor
            .shared
            .state
            .lock()
            .unwrap()
            .reserved_ids
            .is_empty());
    }

    #[test]
    fn failure_and_runner_panic_do_not_discard_following_batches() {
        let mut count = 0;
        let executor = SystemAdjustmentExecutor::with_runners(
            move |action, point| {
                count += 1;
                match count {
                    1 => Err("mock unavailable".into()),
                    2 => panic!("mock panic"),
                    _ => immediate(action, point),
                }
            },
            immediate,
        );
        for id in 1..=3 {
            executor
                .submit(id, action(AdjustmentTarget::Volume, 1), (0, 0))
                .unwrap();
        }
        for id in 1..=3 {
            let event = receive(&executor);
            assert_eq!(event.id, id);
            assert_eq!(event.result.is_ok(), id == 3);
        }
    }

    #[test]
    fn close_rejects_submissions_and_discards_unstarted_work_without_waiting() {
        let (runner, calls, permits) = gated();
        let executor = SystemAdjustmentExecutor::with_runners(runner, immediate);
        let up = action(AdjustmentTarget::Volume, 1);
        executor.submit(1, up, (0, 0)).unwrap();
        calls.recv_timeout(TIMEOUT).unwrap();
        executor.submit(2, up, (0, 0)).unwrap();
        executor.close();
        assert!(executor.submit(3, up, (0, 0)).is_err());
        assert_eq!(executor.try_recv(), Err(TryRecvError::Disconnected));
        let (dropped_tx, dropped_rx) = mpsc::channel();
        thread::spawn(move || {
            drop(executor);
            dropped_tx.send(()).unwrap();
        });
        dropped_rx.recv_timeout(TIMEOUT).unwrap();
        permits.send(()).unwrap();
        // 第一批结束后线程退出；第二批绝不触及 mock runner。
        assert_eq!(
            calls.recv_timeout(TIMEOUT),
            Err(mpsc::RecvTimeoutError::Disconnected)
        );
    }

    #[test]
    fn invalid_delta_does_not_reserve_capacity_or_call_runner() {
        let executor = SystemAdjustmentExecutor::with_runners(
            |_, _| panic!("不应执行无效动作"),
            |_, _| panic!("不应执行无效动作"),
        );
        for delta in [0, -101, 101] {
            assert!(executor
                .submit(1, action(AdjustmentTarget::Volume, delta), (0, 0))
                .is_err());
        }
        assert!(!executor.is_running(1));
        assert!(executor
            .shared
            .state
            .lock()
            .unwrap()
            .reserved_ids
            .is_empty());
    }

    #[test]
    fn negative_pending_merge_saturates_without_changing_direction() {
        let mut batch = Batch {
            id: 1,
            action: action(AdjustmentTarget::Volume, -70),
            point: (0, 0),
        };
        assert!(batch.merge(Batch {
            action: action(AdjustmentTarget::Volume, -70),
            point: (50, 50),
            ..batch
        }));
        assert_eq!(batch.action.delta_percent, -100);
        assert!(!batch.merge(Batch {
            action: action(AdjustmentTarget::Volume, 1),
            ..batch
        }));
        assert_eq!(batch.action.delta_percent, -100);
    }
}
