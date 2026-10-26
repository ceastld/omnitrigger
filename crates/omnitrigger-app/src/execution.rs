//! 外部命令与快捷键的应用层执行边界，不参与 Hook 或核心匹配。
use crate::shortcut::ShortcutAction;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const CAPACITY: usize = 4;
const POLL_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandAction {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
}

impl CommandAction {
    pub fn validate(&self) -> Result<(), String> {
        if self.program.trim().is_empty() {
            return Err("请填写要执行的程序。".into());
        }
        if self.args.len() > 128 {
            return Err("命令参数不能超过 128 项。".into());
        }
        if self
            .working_directory
            .as_ref()
            .is_some_and(|path| path.trim().is_empty())
        {
            return Err("工作目录不能为空；无需指定时请省略该项。".into());
        }
        let fields = std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .chain(self.working_directory.as_deref());
        let mut size = 0usize;
        for field in fields {
            if field.contains('\0') {
                return Err("程序、参数和工作目录不能包含空字符。".into());
            }
            size = size.saturating_add(field.len());
        }
        if size > 64 * 1024 {
            return Err("命令配置不能超过 64 KiB。".into());
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ExecutionEvent {
    /// 产生调用的绑定；一次绑定可以在先前终态被读取后再次调用。
    pub id: u64,
    pub invocation_id: u64,
    pub outcome: ExecutionOutcome,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ExecutionOutcome {
    ShortcutSent,
    Started {
        pid: u32,
    },
    /// 只描述进程退出状态，不代表插件的业务操作已完成。
    Finished {
        code: Option<i32>,
        success: bool,
    },
    Failed {
        message: String,
    },
    /// 进程仍可能运行；这不是终态，不能释放绑定或插件组配额。
    Unknown {
        message: String,
    },
}

struct Request {
    id: u64,
    invocation_id: u64,
    action: ExecutableAction,
}

enum ExecutableAction {
    Command(CommandAction),
    Shortcut(Box<dyn FnOnce() -> Result<(), String> + Send>),
}

struct Running {
    id: u64,
    invocation_id: u64,
    child: Child,
    unknown_notified: bool,
}

struct Reservation {
    binding_id: u64,
    group: Option<String>,
    shortcut: bool,
}

struct Reservations {
    next_id: u64,
    entries: HashMap<u64, Reservation>,
}

/// 无论工作线程正常结束还是 panic，都停止后续接纳；已有配额保留其未知状态。
struct StopOnExit(Arc<AtomicBool>);

impl Drop for StopOnExit {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

pub struct CommandExecutor {
    requests: SyncSender<Request>,
    events: Receiver<ExecutionEvent>,
    /// 终态被 UI 读取后才释放配额，保证结果积压同样有界。
    reserved: Mutex<Reservations>,
    stopping: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    startup_error: Option<String>,
}

impl CommandExecutor {
    pub fn new() -> Self {
        Self::with_worker_name("omnitrigger-commands")
    }

    pub fn shortcuts() -> Self {
        Self::with_worker_name("omnitrigger-shortcuts")
    }

    fn with_worker_name(name: &'static str) -> Self {
        let (requests_tx, requests_rx) = mpsc::sync_channel(CAPACITY);
        // 每个配额最多产生 Started + 一次 Unknown + 终态，不因 UI 未轮询而阻塞。
        let (events_tx, events_rx) = mpsc::sync_channel(CAPACITY * 3);
        let stopping = Arc::new(AtomicBool::new(false));
        let worker_stop = stopping.clone();
        let (worker, startup_error) =
            match thread::Builder::new().name(name.into()).spawn(move || {
                let _exit = StopOnExit(worker_stop.clone());
                run(requests_rx, events_tx, worker_stop);
            }) {
                Ok(worker) => (Some(worker), None),
                Err(error) => (None, Some(format!("无法启动命令执行线程：{error}"))),
            };
        Self {
            requests: requests_tx,
            events: events_rx,
            reserved: Mutex::new(Reservations {
                next_id: 1,
                entries: HashMap::new(),
            }),
            stopping,
            worker,
            startup_error,
        }
    }

    /// 非阻塞提交；同一绑定及全部绑定的待启动、运行和未读终态共用四个配额。
    pub fn submit(&self, id: u64, command: CommandAction) -> Result<(), String> {
        self.submit_group(id, command, None)
    }

    /// 插件组在所有绑定、面板与测试入口间共享单并发；终态未读取前仍占用配额。
    pub fn submit_group(
        &self,
        id: u64,
        command: CommandAction,
        group: Option<String>,
    ) -> Result<(), String> {
        command.validate()?;
        self.submit_request(id, ExecutableAction::Command(command), group)
    }

    pub fn submit_shortcut(&self, id: u64, shortcut: ShortcutAction) -> Result<(), String> {
        shortcut.validate()?;
        let target = omnitrigger_windows::capture_shortcut_target()?;
        self.submit_request(
            id,
            ExecutableAction::Shortcut(Box::new(move || {
                omnitrigger_windows::send_shortcut_to(&shortcut.keys, target)
            })),
            None,
        )
    }

    fn submit_request(
        &self,
        id: u64,
        action: ExecutableAction,
        group: Option<String>,
    ) -> Result<(), String> {
        if group.as_ref().is_some_and(|group| {
            group.trim().is_empty() || group.len() > 256 || group.contains('\0')
        }) {
            return Err("插件并发组名称无效。".into());
        }
        if let Some(error) = &self.startup_error {
            return Err(error.clone());
        }
        if self.stopping.load(Ordering::Acquire)
            || self
                .worker
                .as_ref()
                .is_some_and(|worker| worker.is_finished())
        {
            return Err("命令执行线程已结束，无法接纳新的调用。".into());
        }
        let shortcut = matches!(&action, ExecutableAction::Shortcut(_));
        let mut reserved = self.reserved.lock().map_err(|_| "命令执行状态不可用。")?;
        if reserved
            .entries
            .values()
            .any(|entry| entry.binding_id == id && (!shortcut || !entry.shortcut))
        {
            return Err("此触发器的上次命令仍在执行或等待处理结果。".into());
        }
        if group.as_ref().is_some_and(|group| {
            reserved
                .entries
                .values()
                .any(|entry| entry.group.as_ref() == Some(group))
        }) {
            return Err("此插件动作仍在执行或等待处理结果，请稍后重试。".into());
        }
        if reserved.entries.len() >= CAPACITY {
            return Err("命令执行队列已满，最多同时处理 4 项。".into());
        }
        let invocation_id = reserved.next_id;
        reserved.next_id = invocation_id.checked_add(1).ok_or("命令调用标识已耗尽。")?;
        reserved.entries.insert(
            invocation_id,
            Reservation {
                binding_id: id,
                group,
                shortcut,
            },
        );
        if let Err(error) = self.requests.try_send(Request {
            id,
            invocation_id,
            action,
        }) {
            reserved.entries.remove(&invocation_id);
            return Err(match error {
                mpsc::TrySendError::Full(_) => "命令执行队列已满。".into(),
                mpsc::TrySendError::Disconnected(_) => "命令执行线程已结束。".into(),
            });
        }
        Ok(())
    }

    /// 待启动、运行中或终态尚未读取时均返回 true，供界面禁用重复执行入口。
    pub fn is_running(&self, id: u64) -> bool {
        self.reserved.lock().map_or(true, |reserved| {
            reserved
                .entries
                .values()
                .any(|entry| entry.binding_id == id)
        })
    }

    pub fn is_group_running(&self, group: &str) -> bool {
        self.reserved.lock().map_or(true, |reserved| {
            reserved
                .entries
                .values()
                .any(|entry| entry.group.as_deref() == Some(group))
        })
    }

    pub fn try_recv(&self) -> Result<ExecutionEvent, TryRecvError> {
        let event = self.events.try_recv().inspect_err(|error| {
            if *error == TryRecvError::Disconnected {
                self.stopping.store(true, Ordering::Release);
            }
        })?;
        if matches!(
            event.outcome,
            ExecutionOutcome::Finished { .. }
                | ExecutionOutcome::Failed { .. }
                | ExecutionOutcome::ShortcutSent
        ) {
            if let Ok(mut reserved) = self.reserved.lock() {
                if reserved
                    .entries
                    .get(&event.invocation_id)
                    .is_some_and(|entry| entry.binding_id == event.id)
                {
                    reserved.entries.remove(&event.invocation_id);
                }
            }
        }
        Ok(event)
    }
}

impl Drop for CommandExecutor {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            // CreateProcess 可能正在访问不可用的网络路径；退出 UI 不等待该系统调用。
            // 未结束的 worker 在调用返回后检查 stopping，自行释放其进程句柄。
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}

fn spawn(command: CommandAction) -> std::io::Result<Child> {
    let mut process = Command::new(command.program);
    process
        .args(command.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(directory) = command.working_directory {
        process.current_dir(directory);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        process.creation_flags(0x0800_0000); // CREATE_NO_WINDOW，仅隐藏控制台窗口。
    }
    process.spawn()
}

fn run(requests: Receiver<Request>, events: SyncSender<ExecutionEvent>, stopping: Arc<AtomicBool>) {
    let mut running: Vec<Running> = Vec::with_capacity(CAPACITY);
    while !stopping.load(Ordering::Acquire) {
        let first = match requests.recv_timeout(POLL_INTERVAL) {
            Ok(request) => Some(request),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        for request in first.into_iter().chain(requests.try_iter()) {
            if stopping.load(Ordering::Acquire) {
                break;
            }
            let id = request.id;
            let invocation_id = request.invocation_id;
            let outcome = match request.action {
                ExecutableAction::Shortcut(send_shortcut) => match send_shortcut() {
                    Ok(()) => ExecutionOutcome::ShortcutSent,
                    Err(message) => ExecutionOutcome::Failed { message },
                },
                ExecutableAction::Command(command) => match spawn(command) {
                    Ok(child) => {
                        if stopping.load(Ordering::Acquire) {
                            return;
                        }
                        let pid = child.id();
                        running.push(Running {
                            id,
                            invocation_id,
                            child,
                            unknown_notified: false,
                        });
                        ExecutionOutcome::Started { pid }
                    }
                    Err(error) => ExecutionOutcome::Failed {
                        message: format!("启动程序失败：{error}"),
                    },
                },
            };
            if events
                .send(ExecutionEvent {
                    id,
                    invocation_id,
                    outcome,
                })
                .is_err()
            {
                return;
            }
        }
        let mut index = 0;
        while index < running.len() {
            let outcome = match running[index].child.try_wait() {
                Ok(Some(status)) => ExecutionOutcome::Finished {
                    code: status.code(),
                    success: status.success(),
                },
                Ok(None) => {
                    index += 1;
                    continue;
                }
                Err(error) => {
                    let task = &mut running[index];
                    if !task.unknown_notified {
                        task.unknown_notified = true;
                        if events
                            .send(ExecutionEvent {
                                id: task.id,
                                invocation_id: task.invocation_id,
                                outcome: ExecutionOutcome::Unknown {
                                    message: format!(
                                        "进程状态未知，仍保留执行占用并继续检查：{error}"
                                    ),
                                },
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                    index += 1;
                    continue;
                }
            };
            let task = running.swap_remove(index);
            if events
                .send(ExecutionEvent {
                    id: task.id,
                    invocation_id: task.invocation_id,
                    outcome,
                })
                .is_err()
            {
                return;
            }
        }
    }
    // Child 的析构只关闭进程句柄。外部程序继续运行，不杀进程、不等待 GUI 退出。
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::HashSet, path::PathBuf, sync::atomic::AtomicU64, time::Instant};

    fn scratch() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "omnitrigger-execution-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn fixture(mode: &str, values: &[String]) -> CommandAction {
        let mut args = vec![
            "--ignored".into(),
            "--exact".into(),
            "execution::tests::process_fixture".into(),
            "--nocapture".into(),
            "--skip".into(),
            mode.into(),
        ];
        for value in values {
            args.extend(["--skip".into(), value.clone()]);
        }
        CommandAction {
            program: std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            args,
            working_directory: None,
        }
    }

    fn terminal(executor: &CommandExecutor, id: u64) -> ExecutionOutcome {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match executor.try_recv() {
                Ok(event) => {
                    assert_eq!(event.id, id);
                    if matches!(
                        event.outcome,
                        ExecutionOutcome::Finished { .. }
                            | ExecutionOutcome::Failed { .. }
                            | ExecutionOutcome::ShortcutSent
                    ) {
                        assert!(!executor.is_running(id));
                        return event.outcome;
                    }
                    assert!(executor.is_running(id));
                }
                Err(TryRecvError::Empty) => {}
                Err(error) => panic!("执行线程意外退出：{error}"),
            }
            assert!(Instant::now() < deadline, "等待命令退出超时");
            thread::sleep(POLL_INTERVAL);
        }
    }

    #[test]
    #[ignore = "由执行器测试作为子进程调用"]
    fn process_fixture() {
        let args: Vec<String> = std::env::args().collect();
        let values: Vec<&str> = args
            .windows(2)
            .filter(|pair| pair[0] == "--skip")
            .map(|pair| pair[1].as_str())
            .collect();
        match values[0] {
            "write" => std::fs::write(values[1], values[2]).unwrap(),
            "exit" => std::process::exit(7),
            "wait" | "wait_write" => {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !std::path::Path::new(values[1]).exists() {
                    assert!(Instant::now() < deadline, "子进程等待释放超时");
                    thread::sleep(POLL_INTERVAL);
                }
                if values[0] == "wait_write" {
                    std::fs::write(values[2], "外部程序继续运行").unwrap();
                }
            }
            other => panic!("未知测试模式：{other}"),
        }
    }

    #[test]
    fn preserves_unicode_spaces_and_shell_metacharacters_in_arguments() {
        let output = scratch();
        let payload = "中文 空格 \"quote\" ` $ & | ; < > ( ) % ! \\尾\\";
        let executor = CommandExecutor::new();
        executor
            .submit(
                1,
                fixture(
                    "write",
                    &[output.to_string_lossy().into_owned(), payload.into()],
                ),
            )
            .unwrap();
        assert_eq!(
            terminal(&executor, 1),
            ExecutionOutcome::Finished {
                code: Some(0),
                success: true
            }
        );
        assert_eq!(std::fs::read_to_string(&output).unwrap(), payload);
        std::fs::remove_file(output).unwrap();
    }

    #[test]
    fn reports_missing_program_and_nonzero_exit_and_releases_reservations() {
        let executor = CommandExecutor::new();
        executor
            .submit(
                1,
                CommandAction {
                    program: scratch().to_string_lossy().into_owned(),
                    args: vec![],
                    working_directory: None,
                },
            )
            .unwrap();
        assert!(matches!(
            terminal(&executor, 1),
            ExecutionOutcome::Failed { .. }
        ));
        executor.submit(1, fixture("exit", &[])).unwrap();
        assert_eq!(
            terminal(&executor, 1),
            ExecutionOutcome::Finished {
                code: Some(7),
                success: false
            }
        );
    }

    #[test]
    fn rejects_duplicate_ids_and_caps_all_outstanding_work_at_four() {
        let release = scratch();
        let command = fixture("wait", &[release.to_string_lossy().into_owned()]);
        let executor = CommandExecutor::new();
        for id in 1..=4 {
            assert!(!executor.is_running(id));
            executor.submit(id, command.clone()).unwrap();
            assert!(executor.is_running(id));
        }
        assert!(executor.submit(1, command.clone()).is_err());
        assert!(executor.submit(5, command).is_err());
        assert_eq!(executor.reserved.lock().unwrap().entries.len(), 4);
        std::fs::write(&release, "释放测试进程").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut completed = HashSet::new();
        let mut started = HashSet::new();
        while completed.len() < 4 {
            match executor.try_recv() {
                Ok(event) => match event.outcome {
                    ExecutionOutcome::Started { .. } => {
                        assert!(executor.is_running(event.id));
                        started.insert(event.id);
                    }
                    ExecutionOutcome::Finished { success: true, .. } => {
                        assert!(!executor.is_running(event.id));
                        completed.insert(event.id);
                    }
                    other => panic!("意外进程结果：{other:?}"),
                },
                Err(TryRecvError::Empty) => {}
                Err(error) => panic!("执行线程意外退出：{error}"),
            }
            assert!(Instant::now() < deadline, "等待容量测试完成超时");
            thread::sleep(POLL_INTERVAL);
        }
        assert_eq!(started.len(), 4);
        assert!(executor.reserved.lock().unwrap().entries.is_empty());
        std::fs::remove_file(release).unwrap();
    }

    #[test]
    fn rejects_invalid_commands_before_reserving_capacity() {
        let executor = CommandExecutor::new();
        let mut command = fixture("exit", &[]);
        command.args.push("含\0空字符".into());
        assert!(executor.submit(1, command).is_err());
        assert!(executor.reserved.lock().unwrap().entries.is_empty());
    }

    #[test]
    fn sends_shortcuts_off_thread_and_releases_success_and_failure_reservations() {
        let executor = CommandExecutor::new();
        let success = || {
            ExecutableAction::Shortcut(Box::new(|| {
                assert_eq!(thread::current().name(), Some("omnitrigger-commands"));
                Ok(())
            }))
        };
        executor.submit_request(1, success(), None).unwrap();
        assert_eq!(terminal(&executor, 1), ExecutionOutcome::ShortcutSent);
        executor
            .submit_request(
                1,
                ExecutableAction::Shortcut(Box::new(|| Err("模拟输入被目标窗口拒绝".into()))),
                None,
            )
            .unwrap();
        assert_eq!(
            terminal(&executor, 1),
            ExecutionOutcome::Failed {
                message: "模拟输入被目标窗口拒绝".into(),
            }
        );
        assert!(executor
            .submit_shortcut(2, ShortcutAction { keys: vec![] })
            .is_err());
        assert!(executor.reserved.lock().unwrap().entries.is_empty());
        executor.submit_request(1, success(), None).unwrap();
        assert_eq!(terminal(&executor, 1), ExecutionOutcome::ShortcutSent);
    }

    #[test]
    fn queues_same_binding_shortcuts_in_order_and_releases_each_invocation_independently() {
        let executor = CommandExecutor::new();
        let sent = Arc::new(Mutex::new(Vec::new()));
        for index in 0..CAPACITY {
            let sent = sent.clone();
            executor
                .submit_request(
                    7,
                    ExecutableAction::Shortcut(Box::new(move || {
                        sent.lock().unwrap().push(index);
                        Ok(())
                    })),
                    None,
                )
                .unwrap();
        }
        // 已发送但结果未读的调用也占用配额；命令仍不能与同绑定调用重叠。
        assert!(executor
            .submit_request(7, ExecutableAction::Shortcut(Box::new(|| Ok(()))), None)
            .is_err());
        assert!(executor.submit(7, fixture("exit", &[])).is_err());
        assert_eq!(executor.reserved.lock().unwrap().entries.len(), CAPACITY);
        let deadline = Instant::now() + Duration::from_secs(10);
        for index in 0..CAPACITY {
            let event = loop {
                match executor.try_recv() {
                    Ok(event) => break event,
                    Err(TryRecvError::Empty) => {}
                    Err(error) => panic!("快捷键执行线程意外退出：{error}"),
                }
                assert!(Instant::now() < deadline, "等待快捷键发送结果超时");
                thread::sleep(POLL_INTERVAL);
            };
            assert_eq!(event.id, 7);
            assert_eq!(event.invocation_id, index as u64 + 1);
            assert_eq!(event.outcome, ExecutionOutcome::ShortcutSent);
            let reserved = executor.reserved.lock().unwrap();
            assert_eq!(reserved.entries.len(), CAPACITY - index - 1);
            assert!(!reserved.entries.contains_key(&event.invocation_id));
            drop(reserved);
            assert_eq!(executor.is_running(7), index + 1 < CAPACITY);
        }
        assert_eq!(*sent.lock().unwrap(), (0..CAPACITY).collect::<Vec<_>>());
        executor
            .submit_request(7, ExecutableAction::Shortcut(Box::new(|| Ok(()))), None)
            .unwrap();
        assert_eq!(terminal(&executor, 7), ExecutionOutcome::ShortcutSent);
    }

    #[test]
    fn shares_plugin_group_across_bindings_and_caps_mixed_work_at_four() {
        let release = scratch();
        let command = fixture("wait", &[release.to_string_lossy().into_owned()]);
        let executor = CommandExecutor::new();
        executor
            .submit_group(1, command.clone(), Some("raishot/capture".into()))
            .unwrap();
        assert!(executor.is_group_running("raishot/capture"));
        assert!(executor
            .submit_group(2, command.clone(), Some("raishot/capture".into()))
            .is_err());
        assert!(executor
            .submit_group(1, command.clone(), Some("other/action".into()))
            .is_err());
        executor
            .submit_group(2, command.clone(), Some("other/action".into()))
            .unwrap();
        executor.submit(3, command.clone()).unwrap();
        executor.submit(4, command.clone()).unwrap();
        assert!(executor.submit(5, command).is_err());
        assert_eq!(executor.reserved.lock().unwrap().entries.len(), 4);
        std::fs::write(&release, "释放测试进程").unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut completed = HashSet::new();
        let mut invocations = HashSet::new();
        while completed.len() < 4 {
            match executor.try_recv() {
                Ok(event) => {
                    invocations.insert(event.invocation_id);
                    match event.outcome {
                        ExecutionOutcome::Started { .. } => {
                            assert!(executor.is_running(event.id));
                        }
                        ExecutionOutcome::Finished { success: true, .. } => {
                            assert!(!executor.is_running(event.id));
                            completed.insert(event.id);
                        }
                        other => panic!("意外进程结果：{other:?}"),
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(error) => panic!("执行线程意外退出：{error}"),
            }
            assert!(Instant::now() < deadline, "等待插件组测试完成超时");
            thread::sleep(POLL_INTERVAL);
        }
        assert_eq!(invocations.len(), 4);
        assert!(!executor.is_group_running("raishot/capture"));
        assert!(!executor.is_group_running("other/action"));
        assert!(executor.reserved.lock().unwrap().entries.is_empty());
        std::fs::remove_file(release).unwrap();
    }

    #[test]
    fn releases_plugin_group_after_start_failure_and_nonzero_exit() {
        let executor = CommandExecutor::new();
        let group = "raishot/capture";
        executor
            .submit_group(
                1,
                CommandAction {
                    program: scratch().to_string_lossy().into_owned(),
                    args: vec![],
                    working_directory: None,
                },
                Some(group.into()),
            )
            .unwrap();
        assert!(executor.is_group_running(group));
        assert!(matches!(
            terminal(&executor, 1),
            ExecutionOutcome::Failed { .. }
        ));
        assert!(!executor.is_group_running(group));
        executor
            .submit_group(2, fixture("exit", &[]), Some(group.into()))
            .unwrap();
        assert_eq!(
            terminal(&executor, 2),
            ExecutionOutcome::Finished {
                code: Some(7),
                success: false
            }
        );
        assert!(!executor.is_group_running(group));
    }

    #[test]
    fn unknown_status_retains_group_and_disconnection_stops_new_admission() {
        let (requests, _requests_rx) = mpsc::sync_channel(CAPACITY);
        let (events_tx, events) = mpsc::sync_channel(CAPACITY);
        let executor = CommandExecutor {
            requests,
            events,
            reserved: Mutex::new(Reservations {
                next_id: 43,
                entries: HashMap::from([(
                    42,
                    Reservation {
                        binding_id: 1,
                        group: Some("raishot/capture".into()),
                        shortcut: false,
                    },
                )]),
            }),
            stopping: Arc::new(AtomicBool::new(false)),
            worker: None,
            startup_error: None,
        };
        events_tx
            .send(ExecutionEvent {
                id: 1,
                invocation_id: 42,
                outcome: ExecutionOutcome::Unknown {
                    message: "模拟系统状态读取错误".into(),
                },
            })
            .unwrap();
        assert!(matches!(
            executor.try_recv().unwrap().outcome,
            ExecutionOutcome::Unknown { .. }
        ));
        assert!(executor.is_running(1));
        assert!(executor.is_group_running("raishot/capture"));
        assert!(executor
            .submit_group(2, fixture("exit", &[]), Some("raishot/capture".into()))
            .is_err());

        drop(events_tx);
        assert_eq!(executor.try_recv(), Err(TryRecvError::Disconnected));
        assert!(executor.submit(3, fixture("exit", &[])).is_err());
        assert!(executor.is_group_running("raishot/capture"));
    }

    #[test]
    fn dropping_executor_stops_monitoring_without_killing_external_program() {
        let release = scratch();
        let output = scratch();
        let executor = CommandExecutor::new();
        executor
            .submit(
                1,
                fixture(
                    "wait_write",
                    &[
                        release.to_string_lossy().into_owned(),
                        output.to_string_lossy().into_owned(),
                    ],
                ),
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match executor.try_recv() {
                Ok(ExecutionEvent {
                    outcome: ExecutionOutcome::Started { .. },
                    ..
                }) => break,
                Ok(other) => panic!("意外进程结果：{other:?}"),
                Err(TryRecvError::Empty) => {}
                Err(error) => panic!("执行线程意外退出：{error}"),
            }
            assert!(Instant::now() < deadline, "等待进程启动超时");
            thread::sleep(POLL_INTERVAL);
        }
        let closing = Instant::now();
        drop(executor);
        assert!(closing.elapsed() < Duration::from_secs(1));
        std::fs::write(&release, "释放测试进程").unwrap();
        while !output.exists() {
            assert!(Instant::now() < deadline, "外部程序未继续运行");
            thread::sleep(POLL_INTERVAL);
        }
        assert_eq!(
            std::fs::read_to_string(&output).unwrap(),
            "外部程序继续运行"
        );
        std::fs::remove_file(release).unwrap();
        std::fs::remove_file(output).unwrap();
    }
}
