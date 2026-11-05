//! Hook外的单一快捷键队列接线；仅保存PreparedShortcut并借用原shortcut_executor。
use crate::execution::{
    CommandExecutor, ExecutionEvent, ExecutionOutcome, PreparedShortcut, ShortcutCancelState,
};
use omnitrigger_core::{
    ActiveMouseDispatch, BusyMousePolicy, DispatchEnablement, DroppedMouseDispatch,
    MouseDispatchAdmission, MouseDispatchClass, MouseDispatchControls, MouseDispatchDropReason,
    MouseDispatchError, MouseDispatchQueue, MouseDispatchRequest, MouseDispatchUpdate,
    MAX_DELAYED_DISPATCHES,
};

// 开发入口只读显式计划；没有本选项时不读取文件、不改变旧命令解析。
const PLAN_FLAG: &str = "--mouse-shortcut-queue";
const MAX_PLAN_BYTES: u64 = 16 * 1024;
#[derive(Debug)]
pub struct CliError {
    pub code: &'static str,
    pub message: String,
}
fn cli_error(code: &'static str, message: impl Into<String>) -> CliError {
    CliError {
        code,
        message: message.into(),
    }
}
pub fn reject_cli(error: CliError) -> ! {
    eprintln!(
        "{}",
        serde_json::json!({"status":"rejected","error":{
        "code":error.code,"message":error.message,"effect_state":"not_started"}})
    );
    std::process::exit(1)
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanDto {
    bindings: Vec<u64>,
    policy: PolicyDto,
}
#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum PolicyDto {
    Drop {},
    Delay {
        capacity: usize,
        max_submit_wait_ms: u32,
    },
}

pub fn parse_plan_path(arguments: &[String]) -> Result<Option<std::path::PathBuf>, CliError> {
    let start = usize::from(arguments.first().is_some_and(|arg| !arg.starts_with("--")));
    let arguments = &arguments[start..];
    if !arguments
        .iter()
        .any(|arg| arg == PLAN_FLAG || arg.starts_with("--mouse-shortcut-queue="))
    {
        return Ok(None);
    }
    let mut seen = std::collections::HashSet::new();
    let mut path = None;
    let mut index = 0;
    while index < arguments.len() {
        let flag = arguments[index].as_str();
        if !seen.insert(flag) {
            return Err(cli_error(
                "invalid_mouse_queue_arguments",
                "鼠标快捷键队列启动参数不能重复。",
            ));
        }
        match flag {
            PLAN_FLAG | "--config" => {
                let value = arguments.get(index + 1)
                    .filter(|value| !value.is_empty() && !value.starts_with("--"))
                    .ok_or_else(|| cli_error("invalid_mouse_queue_arguments", format!("{flag} 后需要显式文件路径。")))?;
                if flag == PLAN_FLAG { path = Some(std::path::PathBuf::from(value)); }
                index += 2;
            }
            "--check-config" | "--panel" => index += 1,
            _ => return Err(cli_error("invalid_mouse_queue_arguments",
                "--mouse-shortcut-queue 仅接受普通主窗口启动、可选 --panel 或 --check-config，以及可选 --config 路径；不支持历史窗口、其他操作模式或等号形式。")),
        }
    }
    if seen.contains("--check-config") && seen.contains("--panel") {
        return Err(cli_error(
            "invalid_mouse_queue_arguments",
            "--check-config 不能同时打开面板。",
        ));
    }
    path.map(Some)
        .ok_or_else(|| cli_error("invalid_mouse_queue_arguments", "需要独立的计划文件参数。"))
}

fn decode_plan(bytes: &[u8]) -> Result<Plan, CliError> {
    let dto: PlanDto = serde_json::from_slice(bytes).map_err(|error| {
        cli_error(
            "invalid_mouse_queue_plan",
            format!("鼠标快捷键计划 JSON 无效：{error}"),
        )
    })?;
    let policy = match dto.policy {
        PolicyDto::Drop {} => BusyMousePolicy::Drop,
        PolicyDto::Delay {
            capacity,
            max_submit_wait_ms,
        } => BusyMousePolicy::Delay {
            capacity,
            max_wait_ms: max_submit_wait_ms,
        },
    };
    if dto.bindings.is_empty()
        || dto.bindings.len() > MAX_DELAYED_DISPATCHES
        || dto
            .bindings
            .iter()
            .enumerate()
            .any(|(index, id)| *id == 0 || dto.bindings[..index].contains(id))
    {
        return Err(cli_error(
            "invalid_mouse_queue_bindings",
            "计划需要 1–64 个互不重复的正整数绑定 ID。",
        ));
    }
    MouseDispatchQueue::new(policy).map_err(|_| {
        cli_error(
            "invalid_mouse_queue_policy",
            "Delay 容量须为 1–64，投递等待预算须为 1–60000 毫秒。",
        )
    })?;
    Ok(Plan {
        bindings: dto.bindings,
        policy,
    })
}

pub fn read_plan(path: &std::path::Path) -> Result<Plan, CliError> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|error| {
        cli_error(
            "mouse_queue_plan_read_failed",
            format!("无法打开鼠标快捷键计划：{error}"),
        )
    })?;
    let metadata = file.metadata().map_err(|error| {
        cli_error(
            "mouse_queue_plan_read_failed",
            format!("无法检查鼠标快捷键计划：{error}"),
        )
    })?;
    if !metadata.is_file() || metadata.len() > MAX_PLAN_BYTES {
        return Err(cli_error(
            "invalid_mouse_queue_plan_file",
            "计划必须是至多 16 KiB 的普通 JSON 文件。",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_PLAN_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            cli_error(
                "mouse_queue_plan_read_failed",
                format!("无法读取鼠标快捷键计划：{error}"),
            )
        })?;
    if bytes.len() as u64 > MAX_PLAN_BYTES {
        return Err(cli_error(
            "invalid_mouse_queue_plan_file",
            "计划超过 16 KiB。",
        ));
    }
    decode_plan(&bytes)
}

pub fn supports_binding(binding: &crate::settings::Binding) -> bool {
    matches!(binding.action, crate::settings::ActionSpec::SendShortcut(_))
        && matches!(
            binding.spec.activation,
            omnitrigger_core::Activation::Wheel(_)
                | omnitrigger_core::Activation::Press(omnitrigger_core::Button::Mouse(_))
        )
}
pub fn validate_plan_bindings(
    plan: &Plan,
    bindings: &[crate::settings::Binding],
) -> Result<(), CliError> {
    for id in &plan.bindings {
        let binding = bindings
            .iter()
            .find(|binding| binding.spec.id == *id)
            .ok_or_else(|| {
                cli_error(
                    "mouse_queue_binding_missing",
                    format!("计划中的绑定 #{id} 不存在于实际配置。"),
                )
            })?;
        if !supports_binding(binding) {
            return Err(cli_error(
                "mouse_queue_binding_unsupported",
                format!("绑定 #{id} 必须是鼠标按下或滚轮触发的 SendShortcut。"),
            ));
        }
    }
    Ok(())
}

// Drop含待办/运行/未读终态；Delay预算止于submit，不限制executor内部或输入锁等待。
pub struct Plan {
    pub bindings: Vec<u64>,
    pub policy: BusyMousePolicy,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidBindings,
    Core(MouseDispatchError),
    SerialExhausted,
    MissingPayload,
    InvalidReceipt,
}
impl From<MouseDispatchError> for Error {
    fn from(value: MouseDispatchError) -> Self {
        Self::Core(value)
    }
}
impl Error {
    pub fn message(self) -> &'static str {
        match self {
            Self::InvalidBindings => "鼠标快捷键调度范围无效，未改用其他投递路径。",
            Self::Core(reason) => match reason {
                MouseDispatchError::Closed => "鼠标快捷键队列已关闭，未提交。",
                MouseDispatchError::InvalidPolicy => "鼠标快捷键队列策略无效，未提交。",
                _ => "鼠标快捷键队列状态无效，未自动重试。",
            },
            Self::SerialExhausted => "鼠标快捷键请求编号已耗尽，未提交。",
            Self::MissingPayload | Self::InvalidReceipt => {
                "鼠标快捷键投递状态未知，保留占用且不重试。"
            }
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lease {
    pub token: u64,
    pub binding: u64,
    pub invocation: u64,
}
#[derive(Default)]
pub struct Report {
    pub admission: Option<MouseDispatchAdmission>,
    pub submitted: Option<Lease>,
    pub dropped: Vec<DroppedMouseDispatch>,
    pub cancellation: Option<(Lease, Result<ShortcutCancelState, String>)>,
    pub rejected_before_start: Option<(u64, String)>,
}
impl Report {
    pub fn changed(&self) -> bool {
        self.admission.is_some()
            || self.submitted.is_some()
            || !self.dropped.is_empty()
            || self.cancellation.is_some()
            || self.rejected_before_start.is_some()
    }
    fn merge(&mut self, next: Self) {
        self.admission = next.admission.or(self.admission);
        self.submitted = next.submitted.or(self.submitted);
        self.dropped.extend(next.dropped);
        self.cancellation = next.cancellation.or(self.cancellation.take());
        self.rejected_before_start = next
            .rejected_before_start
            .or(self.rejected_before_start.take());
    }
}
struct Work {
    prepared: Option<PreparedShortcut>,
    // 仅供同一控制流的内存port测试；生产构建没有另一种动作或发送器。
    #[cfg(test)]
    fixture: Option<(u64, Vec<u16>, u32)>,
}
trait Port {
    fn submit(&mut self, binding: u64, work: Work) -> Result<u64, String>;
    fn cancel(&mut self, invocation: u64) -> Result<ShortcutCancelState, String>;
}
struct NativePort<'a>(&'a CommandExecutor);
impl Port for NativePort<'_> {
    fn submit(&mut self, binding: u64, work: Work) -> Result<u64, String> {
        let prepared = work.prepared.ok_or("缺少原生PreparedShortcut，未发送。")?;
        self.0.submit_prepared_shortcut_tracked(binding, prepared)
    }
    fn cancel(&mut self, invocation: u64) -> Result<ShortcutCancelState, String> {
        self.0.cancel_shortcut(invocation)
    }
}
struct Waiting {
    active: ActiveMouseDispatch,
    deadline: Option<u64>,
    work: Work,
}
fn controls(epoch: u64, state: DispatchEnablement) -> MouseDispatchControls<'static> {
    MouseDispatchControls {
        epoch,
        configured: state,
        builtin: &[],
    }
}
pub struct MouseShortcutDispatch {
    bindings: Vec<u64>,
    queue: MouseDispatchQueue,
    serial: u64,
    waiting: Vec<Waiting>,
    ready: Option<Waiting>,
    lease: Option<Lease>,
}
impl MouseShortcutDispatch {
    pub fn new(plan: Plan) -> Result<Self, Error> {
        if plan.bindings.is_empty()
            || plan.bindings.len() > MAX_DELAYED_DISPATCHES
            || plan
                .bindings
                .iter()
                .enumerate()
                .any(|(i, id)| *id == 0 || plan.bindings[..i].contains(id))
        {
            return Err(Error::InvalidBindings);
        }
        Ok(Self {
            bindings: plan.bindings,
            queue: MouseDispatchQueue::new(plan.policy)?,
            serial: 0,
            waiting: Vec::with_capacity(MAX_DELAYED_DISPATCHES),
            ready: None,
            lease: None,
        })
    }
    pub fn owns_binding(&self, binding: u64) -> bool {
        self.bindings.contains(&binding)
    }
    pub fn owns_invocation(&self, invocation: u64) -> bool {
        self.lease
            .is_some_and(|lease| lease.invocation == invocation)
    }
    pub fn offer(
        &mut self,
        binding: u64,
        prepared: PreparedShortcut,
        executor: &CommandExecutor,
        now: u64,
        state: DispatchEnablement,
    ) -> Result<Report, Error> {
        self.offer_port(
            &mut NativePort(executor),
            binding,
            Work {
                prepared: Some(prepared),
                #[cfg(test)]
                fixture: None,
            },
            now,
            state,
        )
    }
    fn offer_port(
        &mut self,
        port: &mut impl Port,
        binding: u64,
        work: Work,
        now: u64,
        state: DispatchEnablement,
    ) -> Result<Report, Error> {
        if !self.owns_binding(binding) {
            return Err(Error::InvalidBindings);
        }
        // 先退休到期等待项；不能让已经过期的payload造成虚假的QueueFull。
        let mut report = self.advance_port(port, now, state)?;
        self.serial = self.serial.checked_add(1).ok_or(Error::SerialExhausted)?;
        // serial也是该不可重用payload的context引用，不是binding或PID。
        let request = MouseDispatchRequest {
            serial: self.serial,
            epoch: self.queue.epoch(),
            binding_id: binding,
            context_id: self.serial,
            class: MouseDispatchClass::Configured,
        };
        let admission = self
            .queue
            .offer(request, now, controls(self.queue.epoch(), state))?;
        match admission {
            MouseDispatchAdmission::Started(active) => {
                self.ready = Some(Waiting {
                    active,
                    deadline: None,
                    work,
                })
            }
            MouseDispatchAdmission::Delayed { token, deadline_ms } => self.waiting.push(Waiting {
                active: ActiveMouseDispatch {
                    token,
                    request,
                    cancellation_requested: false,
                },
                deadline: Some(deadline_ms),
                work,
            }),
            MouseDispatchAdmission::Dropped(_) => {}
        }
        report.admission = Some(admission);
        Ok(report)
    }
    fn apply(
        &mut self,
        mut update: MouseDispatchUpdate,
        port: &mut impl Port,
        now: u64,
        state: DispatchEnablement,
    ) -> Result<Report, Error> {
        let mut report = Report::default();
        // ready尚未提交时可确认为未开始；每次complete最多推进一个，最多65个payload。
        for _ in 0..=MAX_DELAYED_DISPATCHES {
            for dropped in update.dropped.drain(..) {
                let at = self
                    .waiting
                    .iter()
                    .position(|entry| entry.active.token == dropped.token)
                    .ok_or(Error::MissingPayload)?;
                self.waiting.remove(at);
                report.dropped.push(dropped);
            }
            if let Some(active) = update.started.take() {
                let at = self
                    .waiting
                    .iter()
                    .position(|entry| entry.active.token == active.token)
                    .ok_or(Error::MissingPayload)?;
                self.ready = Some(self.waiting.remove(at));
            }
            if let Some(token) = update.cancel_active.take() {
                if let Some(lease) = self.lease.filter(|lease| lease.token == token) {
                    report.cancellation = Some((lease, port.cancel(lease.invocation)));
                } else if self
                    .ready
                    .as_ref()
                    .is_some_and(|entry| entry.active.token == token)
                {
                    let ready = self.ready.take().ok_or(Error::MissingPayload)?;
                    report.dropped.push(DroppedMouseDispatch {
                        token,
                        request: ready.active.request,
                        reason: MouseDispatchDropReason::Cancelled,
                    });
                    update =
                        self.queue
                            .complete(token, now, controls(self.queue.epoch(), state))?;
                    continue;
                }
                // 投递回执异常且无lease时不猜ID、不complete；保留有界占用。
            }
            return Ok(report);
        }
        Err(Error::MissingPayload)
    }
    pub fn pump_one(
        &mut self,
        executor: &CommandExecutor,
        now: u64,
        state: DispatchEnablement,
    ) -> Result<Report, Error> {
        self.pump_port(&mut NativePort(executor), now, state)
    }
    fn advance_port(
        &mut self,
        port: &mut impl Port,
        now: u64,
        state: DispatchEnablement,
    ) -> Result<Report, Error> {
        let update = self
            .queue
            .advance(now, controls(self.queue.epoch(), state))?;
        let mut report = self.apply(update, port, now, state)?;
        if self
            .ready
            .as_ref()
            .is_some_and(|ready| ready.deadline.is_some_and(|deadline| now >= deadline))
        {
            let ready = self.ready.take().ok_or(Error::MissingPayload)?;
            report.dropped.push(DroppedMouseDispatch {
                token: ready.active.token,
                request: ready.active.request,
                reason: MouseDispatchDropReason::Expired,
            });
            let next = self.queue.complete(
                ready.active.token,
                now,
                controls(self.queue.epoch(), state),
            )?;
            report.merge(self.apply(next, port, now, state)?);
        }
        Ok(report)
    }
    fn pump_port(
        &mut self,
        port: &mut impl Port,
        now: u64,
        state: DispatchEnablement,
    ) -> Result<Report, Error> {
        let mut report = self.advance_port(port, now, state)?;
        let Some(ready) = self.ready.take() else {
            return Ok(report);
        };
        let token = ready.active.token;
        let binding = ready.active.request.binding_id;
        match port.submit(binding, ready.work) {
            Ok(invocation) if invocation != 0 => {
                let lease = Lease {
                    token,
                    binding,
                    invocation,
                };
                self.lease = Some(lease);
                report.submitted = Some(lease);
            }
            Ok(_) => return Err(Error::InvalidReceipt), // 未知是否接纳，active保留，不重试。
            Err(message) => {
                // 现有submit_prepared的Err均在成功try_send前；只在此明确未开始时释放。
                report.rejected_before_start = Some((binding, message));
                let next = self
                    .queue
                    .complete(token, now, controls(self.queue.epoch(), state))?;
                report.merge(self.apply(next, port, now, state)?);
            }
        }
        Ok(report)
    }
    pub fn observe(
        &mut self,
        from_shortcut_executor: bool,
        event: &ExecutionEvent,
        executor: &CommandExecutor,
        now: u64,
        state: DispatchEnablement,
    ) -> Result<Report, Error> {
        self.observe_port(
            from_shortcut_executor,
            event,
            &mut NativePort(executor),
            now,
            state,
        )
    }
    fn observe_port(
        &mut self,
        from_shortcut_executor: bool,
        event: &ExecutionEvent,
        port: &mut impl Port,
        now: u64,
        state: DispatchEnablement,
    ) -> Result<Report, Error> {
        let Some(lease) = self.lease.filter(|lease| {
            from_shortcut_executor
                && lease.binding == event.id
                && lease.invocation == event.invocation_id
        }) else {
            return Ok(Report::default());
        };
        if !matches!(
            &event.outcome,
            ExecutionOutcome::ShortcutSent | ExecutionOutcome::Failed { .. }
        ) {
            return Ok(Report::default());
        }
        // 事件已由原executor.try_recv消费，sender及其有界收尾尝试已返回；不是业务成功确认。
        let update = self
            .queue
            .complete(lease.token, now, controls(self.queue.epoch(), state))?;
        self.lease = None;
        self.apply(update, port, now, state)
    }
    pub fn cancel_rule(
        &mut self,
        binding: u64,
        executor: &CommandExecutor,
        now: u64,
        state: DispatchEnablement,
    ) -> Result<Report, Error> {
        self.cancel_rule_port(binding, &mut NativePort(executor), now, state)
    }
    fn cancel_rule_port(
        &mut self,
        binding: u64,
        port: &mut impl Port,
        now: u64,
        state: DispatchEnablement,
    ) -> Result<Report, Error> {
        let mut contexts: Vec<u64> = self
            .waiting
            .iter()
            .filter(|entry| entry.active.request.binding_id == binding)
            .map(|entry| entry.active.request.context_id)
            .collect();
        if let Some(active) = self
            .queue
            .active()
            .filter(|active| active.request.binding_id == binding)
        {
            contexts.push(active.request.context_id);
        }
        let mut report = Report::default();
        for context in contexts {
            let update =
                self.queue
                    .invalidate_context(context, now, controls(self.queue.epoch(), state))?;
            report.merge(self.apply(update, port, now, state)?);
        }
        Ok(report)
    }
    pub fn cancel_all(
        &mut self,
        executor: &CommandExecutor,
        now: u64,
        state: DispatchEnablement,
    ) -> Result<Report, Error> {
        self.cancel_all_port(&mut NativePort(executor), now, state)
    }
    fn cancel_all_port(
        &mut self,
        port: &mut impl Port,
        now: u64,
        state: DispatchEnablement,
    ) -> Result<Report, Error> {
        let update = self.queue.cancel().update;
        self.apply(update, port, now, state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    include!("mouse_shortcut_dispatch_tests.rs");
}
