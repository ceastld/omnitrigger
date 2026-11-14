//! 由打开界面前捕获的原目标执行一次明确发送。正文只在后台临时持有。

use crate::clipboard_history_host::WithClipboardHistory;
use crate::clipboard_history_store::{ClipboardHistoryStore, HistorySummary};
use crate::desktop_host::DesktopHost;
use omnitrigger_actions::{ActionError, CancellationToken, NativeHost};
use omnitrigger_windows::{ScriptTarget, ScriptTargetSnapshot, ShortcutTarget};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

// 仅限本进程已验证来源的私有内存目标；不改变CLI上下文的30秒重放限制。
const TARGET_LIFETIME: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InputEffect {
    NotStarted,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SendFailure {
    pub code: &'static str,
    pub effect: InputEffect,
}

impl SendFailure {
    fn before(code: &'static str) -> Self {
        Self {
            code,
            effect: InputEffect::NotStarted,
        }
    }

    pub fn message(self) -> &'static str {
        if self.effect == InputEffect::Unknown {
            return "输入结果尚不确定，请核对原窗口；不会自动重试。";
        }
        match self.code {
            "target_expired" => "原窗口信息已过期，请从原窗口重新打开文本历史。",
            "target_is_own_window" => "不能向 OmniTrigger 自身窗口发送。",
            "target_identity_changed" => "原窗口身份已变化或无法核验，请从原窗口重新打开文本历史。",
            "target_changed" => "原窗口或前台核验未完成，未提交文本。请回到历史窗口后明确重试。",
            "history_selection_changed" => "选中项已变化，请刷新后重新选择。",
            "history_text_empty" => "空文本无需发送。",
            "cancelled" => "发送已取消。",
            "send_busy" => "已有发送尚未结束。",
            _ => "原窗口不可用或前台已变化，未提交文本。",
        }
    }

    fn invalidates_target(self) -> bool {
        self.effect == InputEffect::NotStarted
            && matches!(
                self.code,
                "target_identity_changed" | "target_is_own_window"
            )
    }

    pub fn needs_refresh(self) -> bool {
        self.effect == InputEffect::Unknown || self.code == "history_selection_changed"
    }
}

#[derive(Clone, Copy)]
struct SendTime {
    instant: Instant,
    unix_ms: u64,
}

impl SendTime {
    fn now() -> Result<Self, SendFailure> {
        let unix_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()
            .and_then(|value| value.as_millis().try_into().ok())
            .ok_or_else(|| SendFailure::before("target_expired"))?;
        Ok(Self {
            instant: Instant::now(),
            unix_ms,
        })
    }
}

#[derive(Clone)]
pub(super) struct HistoryLaunchTarget {
    snapshot: ScriptTargetSnapshot,
    own_process: u32,
    captured: Instant,
    expires: Instant,
}

impl HistoryLaunchTarget {
    /// 只在GPUI窗口出现前调用；复用现有ShortcutTarget身份与目标上下文，不读剪贴板。
    pub fn capture(original: ShortcutTarget) -> Result<Self, SendFailure> {
        let captured = Instant::now();
        let target = ScriptTarget::capture(original)
            .map_err(|_| SendFailure::before("target_unavailable"))?;
        target
            .ensure_foreground()
            .map_err(|_| SendFailure::before("target_changed"))?;
        let snapshot = target
            .snapshot()
            .map_err(|_| SendFailure::before("target_changed"))?;
        let own_process = std::process::id();
        let info = target
            .info()
            .map_err(|_| SendFailure::before("target_changed"))?;
        if snapshot.process_id == own_process
            || info.process_name.eq_ignore_ascii_case("omnitrigger.exe")
        {
            return Err(SendFailure::before("target_is_own_window"));
        }
        let context = crate::action_target_context::export(snapshot)
            .map_err(|_| SendFailure::before("target_unavailable"))?;
        let result =
            Self::from_captured(snapshot, context, own_process, captured, SendTime::now()?)?;
        target
            .ensure_foreground()
            .map_err(|_| SendFailure::before("target_changed"))?;
        Ok(result)
    }

    /// 来自本次Host私有快照；复验完整身份，不捕获当前前台。
    pub fn from_host_snapshot(
        snapshot: ScriptTargetSnapshot,
        accepted: Instant,
    ) -> Result<Self, SendFailure> {
        let target = ScriptTarget::from_snapshot(snapshot)
            .map_err(|_| SendFailure::before("target_changed"))?;
        let info = target
            .info()
            .map_err(|_| SendFailure::before("target_changed"))?;
        let own_process = std::process::id();
        if snapshot.process_id == own_process
            || info.process_name.eq_ignore_ascii_case("omnitrigger.exe")
        {
            return Err(SendFailure::before("target_is_own_window"));
        }
        Self::from_verified_snapshot(snapshot, own_process, accepted, SendTime::now()?)
    }

    /// 捕获时用原DTO规则验证来源，然后只保留不可从脚本反序列化的私有快照。
    fn from_captured(
        snapshot: ScriptTargetSnapshot,
        context: Value,
        own_process: u32,
        captured: Instant,
        now: SendTime,
    ) -> Result<Self, SendFailure> {
        let validated = crate::action_target_context::validate_value(context, now.unix_ms)
            .map_err(|_| SendFailure::before("target_expired"))?;
        if validated != snapshot {
            return Err(SendFailure::before("target_changed"));
        }
        Self::from_verified_snapshot(snapshot, own_process, captured, now)
    }

    fn from_verified_snapshot(
        snapshot: ScriptTargetSnapshot,
        own_process: u32,
        captured: Instant,
        now: SendTime,
    ) -> Result<Self, SendFailure> {
        let expires = captured
            .checked_add(TARGET_LIFETIME)
            .ok_or_else(|| SendFailure::before("target_expired"))?;
        let result = Self {
            snapshot,
            own_process,
            captured,
            expires,
        };
        result.validate(now)?;
        Ok(result)
    }

    fn validate(&self, now: SendTime) -> Result<(), SendFailure> {
        if now.instant < self.captured || now.instant >= self.expires {
            return Err(SendFailure::before("target_expired"));
        }
        if self.own_process == 0 || self.snapshot.process_id == self.own_process {
            return Err(SendFailure::before("target_is_own_window"));
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(super) struct HistorySendSelection {
    pub summary: HistorySummary,
    pub revision: String,
}

pub(super) struct HistorySendSession {
    target: Option<HistoryLaunchTarget>,
    ui_window: Option<isize>,
    pending: Option<(u64, String, CancellationToken)>,
    expired: bool,
    invalidated: bool,
}

impl HistorySendSession {
    pub fn new(target: Option<HistoryLaunchTarget>, ui_window: Option<isize>) -> Self {
        let ui_window = ui_window.filter(|window| *window > 0);
        let target =
            target.filter(|target| ui_window != Some(target.snapshot.window_handle as isize));
        Self {
            target,
            ui_window,
            pending: None,
            expired: false,
            invalidated: false,
        }
    }

    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }

    fn cancelling(&self) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|(_, _, cancellation)| cancellation.is_cancelled())
    }

    pub fn action_label(&self) -> &'static str {
        if self.cancelling() {
            "取消中…"
        } else if self.pending() {
            "取消发送"
        } else {
            "发送选中项"
        }
    }

    pub fn action_enabled(&self, selection_ready: bool) -> bool {
        if self.pending() {
            !self.cancelling()
        } else {
            selection_ready && self.ready()
        }
    }

    pub fn ready(&self) -> bool {
        SendTime::now().is_ok_and(|now| self.ready_at(now))
    }

    fn ready_at(&self, now: SendTime) -> bool {
        !self.invalidated
            && !self.expired
            && self.pending.is_none()
            && self.ui_window.is_some()
            && self
                .target
                .as_ref()
                .is_some_and(|target| target.validate(now).is_ok())
    }

    pub fn poll(&mut self) -> bool {
        let expired = self
            .target
            .as_ref()
            .is_some_and(|target| match SendTime::now() {
                Ok(now) => target.validate(now).is_err(),
                Err(_) => true,
            });
        if !self.expired && expired {
            self.expired = true;
            self.cancel();
            return true;
        }
        false
    }

    pub fn status(&self) -> &'static str {
        if self.pending() {
            if self.cancelling() {
                "正在取消发送，等待后台确认…"
            } else {
                "正在向原窗口提交选中的文本…"
            }
        } else if self.invalidated {
            "本次原窗口身份已不可用，请从原窗口重新打开文本历史。"
        } else if self.expired {
            "原窗口信息已过期，请从原窗口重新打开文本历史。"
        } else if self.target.is_none() || self.ui_window.is_none() {
            "本次未保留可发送的原窗口，仍可查询与删除历史。"
        } else {
            "选中后点击发送，只向本次打开前的原窗口提交。"
        }
    }

    pub fn begin(
        &mut self,
        id: u64,
        selected: HistorySendSelection,
    ) -> Result<HistorySendRequest, SendFailure> {
        if self.pending.is_some() {
            return Err(SendFailure::before("send_busy"));
        }
        if self.invalidated {
            return Err(SendFailure::before("target_identity_changed"));
        }
        if self.expired {
            return Err(SendFailure::before("target_expired"));
        }
        let target = self
            .target
            .clone()
            .ok_or_else(|| SendFailure::before("target_unavailable"))?;
        let ui_window = self
            .ui_window
            .ok_or_else(|| SendFailure::before("target_unavailable"))?;
        target.validate(SendTime::now()?)?;
        let cancellation = CancellationToken::new();
        self.pending = Some((id, selected.summary.id.clone(), cancellation.clone()));
        Ok(HistorySendRequest {
            target,
            ui_window,
            selected,
            cancellation,
        })
    }

    pub fn selection_changing(&self, next_id: &str) {
        if let Some((_, _, cancellation)) = self
            .pending
            .as_ref()
            .filter(|(_, id, _)| id.as_str() != next_id)
        {
            cancellation.cancel();
        }
    }

    pub fn cancel(&self) {
        if let Some((_, _, cancellation)) = &self.pending {
            cancellation.cancel();
        }
    }

    pub fn complete(&mut self, id: u64, result: &Result<SendReceipt, SendFailure>) {
        if self
            .pending
            .as_ref()
            .is_none_or(|(pending, _, _)| *pending != id)
        {
            return;
        }
        self.finish(id);
        if result
            .as_ref()
            .err()
            .is_some_and(|error| error.invalidates_target())
        {
            self.target = None;
            self.invalidated = true;
        }
    }

    pub fn finish(&mut self, id: u64) {
        if self
            .pending
            .as_ref()
            .is_some_and(|(pending, _, _)| *pending == id)
        {
            self.pending = None;
        }
    }
}

impl Drop for HistorySendSession {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub(super) struct HistorySendRequest {
    target: HistoryLaunchTarget,
    ui_window: isize,
    selected: HistorySendSelection,
    cancellation: CancellationToken,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SendReceipt {
    pub utf16_units: usize,
}

impl SendReceipt {
    pub fn message(self) -> &'static str {
        if self.utf16_units == 0 {
            "未提交空文本。"
        } else {
            "已向原窗口提交文本。"
        }
    }
}

trait SendPort: NativeHost {
    fn restore_original(
        &self,
        ui_window: isize,
        token: &CancellationToken,
        deadline: Instant,
    ) -> Result<(), SendFailure>;
}

struct DesktopSendPort {
    target: ScriptTarget,
    host: WithClipboardHistory<DesktopHost, Arc<ClipboardHistoryStore>>,
}

impl NativeHost for DesktopSendPort {
    fn call(
        &self,
        method: &str,
        params: Value,
        token: &CancellationToken,
    ) -> Result<Value, ActionError> {
        self.host.call(method, params, token)
    }

    fn call_with_deadline(
        &self,
        method: &str,
        params: Value,
        token: &CancellationToken,
        deadline: Instant,
    ) -> Result<Value, ActionError> {
        self.host
            .call_with_deadline(method, params, token, deadline)
    }
}

impl SendPort for DesktopSendPort {
    fn restore_original(
        &self,
        ui_window: isize,
        token: &CancellationToken,
        deadline: Instant,
    ) -> Result<(), SendFailure> {
        if token.is_cancelled() {
            return Err(SendFailure::before("cancelled"));
        }
        if Instant::now() >= deadline {
            return Err(SendFailure::before("target_expired"));
        }
        if self.target.ensure_foreground().is_ok() {
            return Ok(());
        }
        // 此处只核验调用来源仍是本进程的那一个界面窗口，绝不替换原目标。
        // HWND被其他进程复用或用户已切到第三个应用时，不尝试交还前台。
        let source = omnitrigger_windows::capture_matching_shortcut_target(ui_window)
            .and_then(ScriptTarget::capture)
            .map_err(|_| SendFailure::before("target_changed"))?;
        let source_identity = source
            .snapshot()
            .map_err(|_| SendFailure::before("target_changed"))?;
        if source_identity.process_id != std::process::id() {
            return Err(SendFailure::before("target_changed"));
        }
        source
            .ensure_foreground()
            .map_err(|_| SendFailure::before("target_changed"))?;
        if token.is_cancelled() {
            return Err(SendFailure::before("cancelled"));
        }
        if Instant::now() >= deadline {
            return Err(SendFailure::before("target_expired"));
        }
        if !self
            .target
            .restore_foreground_if(ui_window)
            .map_err(|_| SendFailure::before("target_changed"))?
        {
            return Err(SendFailure::before("target_changed"));
        }
        self.target
            .ensure_foreground()
            .map_err(|_| SendFailure::before("target_changed"))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ItemDto {
    id: String,
    text: String,
    created_at: String,
    revision: String,
}

impl HistorySendRequest {
    /// 后台专用入口；只复用既有Host发送，不创建另一套输入或剪贴板通道。
    pub fn run(self, store: Arc<ClipboardHistoryStore>) -> Result<SendReceipt, SendFailure> {
        self.check(SendTime::now()?)?;
        let target = ScriptTarget::from_snapshot(self.target.snapshot)
            .map_err(|_| SendFailure::before("target_identity_changed"))?;
        let desktop = DesktopHost::from_snapshot(self.target.snapshot)
            .map_err(|_| SendFailure::before("target_identity_changed"))?;
        let port = DesktopSendPort {
            target,
            host: WithClipboardHistory::new(desktop, Some(store)),
        };
        self.execute(&port, SendTime::now)
    }

    fn check(&self, now: SendTime) -> Result<(), SendFailure> {
        if self.cancellation.is_cancelled() {
            return Err(SendFailure::before("cancelled"));
        }
        self.target.validate(now)?;
        if self.ui_window <= 0 || self.ui_window as usize == self.target.snapshot.window_handle {
            return Err(SendFailure::before("target_is_own_window"));
        }
        Ok(())
    }

    fn host_target(
        &self,
        port: &impl SendPort,
        clock: &mut impl FnMut() -> Result<SendTime, SendFailure>,
    ) -> Result<(), SendFailure> {
        self.check(clock()?)?;
        let value = port
            .call_with_deadline(
                "target.context",
                json!({}),
                &self.cancellation,
                self.target.expires,
            )
            .map_err(|error| {
                SendFailure::before(if error.code == "target_changed" {
                    // 既有DesktopHost的target.context用此代码报告snapshot身份核验失败。
                    "target_identity_changed"
                } else if self.cancellation.is_cancelled() {
                    "cancelled"
                } else {
                    "target_changed"
                })
            })?;
        // Host在调用内生成到期时间，须使用调用后的时间验证30秒上限。
        let now = clock()?;
        self.check(now)?;
        let snapshot = crate::action_target_context::validate_value(value, now.unix_ms)
            .map_err(|_| SendFailure::before("target_changed"))?;
        if snapshot.process_id == self.target.own_process {
            return Err(SendFailure::before("target_is_own_window"));
        }
        if snapshot != self.target.snapshot {
            return Err(SendFailure::before("target_identity_changed"));
        }
        Ok(())
    }

    fn selected_text(&self, port: &impl SendPort) -> Result<String, SendFailure> {
        let value = port
            .call_with_deadline(
                "history.get",
                json!({"id":self.selected.summary.id}),
                &self.cancellation,
                self.target.expires,
            )
            .map_err(|_| {
                SendFailure::before(if self.cancellation.is_cancelled() {
                    "cancelled"
                } else {
                    "history_selection_changed"
                })
            })?;
        let item: ItemDto = serde_json::from_value(value)
            .map_err(|_| SendFailure::before("history_selection_changed"))?;
        if item.revision != self.selected.revision
            || !self
                .selected
                .summary
                .matches_item(&item.id, &item.created_at, &item.text)
            || item.text.contains('\0')
            || self.selected.summary.utf16_units > 65_536
        {
            return Err(SendFailure::before("history_selection_changed"));
        }
        if item.text.is_empty() {
            return Err(SendFailure::before("history_text_empty"));
        }
        Ok(item.text)
    }

    fn execute(
        &self,
        port: &impl SendPort,
        mut clock: impl FnMut() -> Result<SendTime, SendFailure>,
    ) -> Result<SendReceipt, SendFailure> {
        self.check(clock()?)?;
        self.host_target(port, &mut clock)?;
        let first = self.selected_text(port)?;
        self.check(clock()?)?;
        self.host_target(port, &mut clock)?;
        self.check(clock()?)?;
        port.restore_original(self.ui_window, &self.cancellation, self.target.expires)?;
        self.check(clock()?)?;
        let text = self.selected_text(port)?;
        if text != first {
            return Err(SendFailure::before("history_selection_changed"));
        }
        self.check(clock()?)?;
        self.host_target(port, &mut clock)?;
        self.check(clock()?)?;
        let result = port
            .call_with_deadline(
                "keyboard.text",
                json!({"text":text}),
                &self.cancellation,
                self.target.expires,
            )
            .map_err(|error| SendFailure {
                code: match error.code.as_str() {
                    "cancelled" => "cancelled",
                    "target_changed" => "target_changed",
                    _ => "input_failed",
                },
                effect: if error.effect_state.as_deref() == Some("not_started") {
                    InputEffect::NotStarted
                } else {
                    InputEffect::Unknown
                },
            })?;
        // 既有keyboard Host只保证目标复验和取消；deadline不能冒充输入期间的硬时限。
        if clock().and_then(|now| self.check(now)).is_err()
            || result.as_object().is_none_or(|value| value.len() != 1)
            || result.get("submitted") != Some(&Value::Bool(true))
        {
            return Err(SendFailure {
                code: "input_result_unknown",
                effect: InputEffect::Unknown,
            });
        }
        Ok(SendReceipt {
            utf16_units: self.selected.summary.utf16_units,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct FakePort {
        calls: Mutex<Vec<&'static str>>,
        contexts: Mutex<Vec<Value>>,
        items: Mutex<Vec<Value>>,
        restore: bool,
        keyboard: Result<Value, ActionError>,
        sent: Mutex<Vec<String>>,
    }

    fn snapshot() -> ScriptTargetSnapshot {
        ScriptTargetSnapshot {
            window_handle: 123,
            process_id: 7,
            thread_id: 8,
            process_created_file_time: 134_386_985_330_123_456,
        }
    }

    fn context(expires: u64) -> Value {
        let value = snapshot();
        json!({"window_handle":value.window_handle.to_string(),"process_id":value.process_id,
            "thread_id":value.thread_id,"process_created_file_time":value.process_created_file_time.to_string(),
            "expires_at_unix_ms":expires})
    }

    fn time() -> SendTime {
        SendTime {
            instant: Instant::now(),
            unix_ms: 1000,
        }
    }

    fn launch(now: SendTime) -> HistoryLaunchTarget {
        HistoryLaunchTarget::from_captured(
            snapshot(),
            context(now.unix_ms + 30_000),
            99,
            now.instant,
            now,
        )
        .unwrap()
    }

    fn selected() -> HistorySendSelection {
        HistorySendSelection {
            summary: HistorySummary::from_text("1", "10", " A😀\n"),
            revision: "2".into(),
        }
    }

    fn request(now: SendTime) -> HistorySendRequest {
        HistorySendRequest {
            target: launch(now),
            ui_window: 456,
            selected: selected(),
            cancellation: CancellationToken::new(),
        }
    }

    fn port() -> FakePort {
        FakePort {
            calls: Mutex::new(Vec::new()),
            contexts: Mutex::new(vec![context(31_000)]),
            items: Mutex::new(vec![
                json!({"id":"1","text":" A😀\n","createdAt":"10","revision":"2"}),
            ]),
            restore: true,
            keyboard: Ok(json!({"submitted":true})),
            sent: Mutex::new(Vec::new()),
        }
    }

    fn next(values: &Mutex<Vec<Value>>) -> Value {
        let mut values = values.lock().unwrap();
        if values.len() > 1 {
            values.remove(0)
        } else {
            values[0].clone()
        }
    }

    impl NativeHost for FakePort {
        fn call(
            &self,
            method: &str,
            params: Value,
            _: &CancellationToken,
        ) -> Result<Value, ActionError> {
            match method {
                "target.context" => {
                    self.calls.lock().unwrap().push("target.context");
                    Ok(next(&self.contexts))
                }
                "history.get" => {
                    assert_eq!(params, json!({"id":"1"}));
                    self.calls.lock().unwrap().push("history.get");
                    Ok(next(&self.items))
                }
                "keyboard.text" => {
                    self.calls.lock().unwrap().push("keyboard.text");
                    self.sent
                        .lock()
                        .unwrap()
                        .push(params["text"].as_str().unwrap().into());
                    self.keyboard.clone()
                }
                _ => panic!("未声明的合成Host方法"),
            }
        }
    }

    impl SendPort for FakePort {
        fn restore_original(
            &self,
            window: isize,
            _: &CancellationToken,
            _: Instant,
        ) -> Result<(), SendFailure> {
            assert_eq!(window, 456);
            self.calls.lock().unwrap().push("restore.original");
            if self.restore {
                Ok(())
            } else {
                Err(SendFailure::before("target_changed"))
            }
        }
    }

    #[test]
    fn trusted_memory_target_expiry_is_monotonic_after_capture_validation() {
        let now = time();
        let target = launch(now);
        assert_eq!(
            target.expires.duration_since(target.captured),
            Duration::from_secs(300)
        );
        assert!(target.validate(now).is_ok());
        assert!(target
            .validate(SendTime {
                instant: now.instant + Duration::from_secs(31),
                unix_ms: 32_000
            })
            .is_ok());
        // 墙钟调整不会重放或延长已保存在内存中的目标；发送的fresh DTO另行检查。
        assert!(target
            .validate(SendTime {
                unix_ms: 999,
                ..now
            })
            .is_ok());
        assert!(target
            .validate(SendTime {
                instant: now.instant + TARGET_LIFETIME,
                ..now
            })
            .is_err());
    }

    #[test]
    fn own_process_and_own_window_cannot_send_or_restore() {
        let now = time();
        let mut request = request(now);
        request.target.own_process = 7;
        let host = port();
        assert_eq!(
            request.execute(&host, || Ok(now)).unwrap_err().code,
            "target_is_own_window"
        );
        assert!(host.calls.lock().unwrap().is_empty());
        request.target.own_process = 99;
        request.ui_window = 123;
        assert!(request.execute(&host, || Ok(now)).is_err());
        assert!(host.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn cancelled_or_expired_request_has_no_host_calls() {
        let now = time();
        for cancelled in [false, true] {
            let request = request(now);
            if cancelled {
                request.cancellation.cancel();
            }
            let host = port();
            assert!(request
                .execute(&host, || Ok(SendTime {
                    instant: now.instant + TARGET_LIFETIME,
                    ..now
                }))
                .is_err());
            assert!(host.calls.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn first_selection_read_is_checked_before_foreground_restore() {
        let now = time();
        let host = port();
        host.items.lock().unwrap()[0]["revision"] = json!("3");
        let error = request(now).execute(&host, || Ok(now)).unwrap_err();
        assert_eq!(error.code, "history_selection_changed");
        assert_eq!(
            *host.calls.lock().unwrap(),
            ["target.context", "history.get"]
        );
    }

    #[test]
    fn foreground_switch_to_another_app_refuses_restore_and_send() {
        let now = time();
        let mut host = port();
        host.restore = false;
        assert_eq!(
            request(now).execute(&host, || Ok(now)).unwrap_err().effect,
            InputEffect::NotStarted
        );
        assert!(host.sent.lock().unwrap().is_empty());
        assert_eq!(host.calls.lock().unwrap().last(), Some(&"restore.original"));
    }

    #[test]
    fn selection_is_read_again_after_foreground_restore() {
        let now = time();
        let host = port();
        let mut second = host.items.lock().unwrap()[0].clone();
        second["revision"] = json!("3");
        host.items.lock().unwrap().push(second);
        assert_eq!(
            request(now).execute(&host, || Ok(now)).unwrap_err().code,
            "history_selection_changed"
        );
        assert!(host.sent.lock().unwrap().is_empty());
        assert_eq!(
            host.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|method| **method == "history.get")
                .count(),
            2
        );
    }

    #[test]
    fn reset_with_identical_preview_and_revision_never_submits_replacement_text() {
        let now = time();
        let old = format!("{}旧", "界".repeat(64));
        let new = format!("{}新", "界".repeat(64));
        let item = |text: &str| json!({"id":"1","text":text,"createdAt":"10","revision":"2"});
        for replaced_after_restore in [false, true] {
            let host = port();
            let mut request = request(now);
            request.selected.summary = HistorySummary::from_text("1", "10", &old);
            *host.items.lock().unwrap() = if replaced_after_restore {
                vec![item(&old), item(&new)]
            } else {
                vec![item(&new)]
            };
            let failure = request.execute(&host, || Ok(now)).unwrap_err();
            assert_eq!(failure.code, "history_selection_changed");
            assert_eq!(failure.effect, InputEffect::NotStarted);
            assert!(host.sent.lock().unwrap().is_empty());
            let calls = host.calls.lock().unwrap();
            assert_eq!(
                calls.iter().filter(|call| **call == "history.get").count(),
                if replaced_after_restore { 2 } else { 1 }
            );
            assert!(!calls.contains(&"keyboard.text"));
        }
    }

    #[test]
    fn changed_process_creation_time_is_revalidated_before_send() {
        let now = time();
        let host = port();
        let mut changed = context(31_000);
        changed["process_created_file_time"] = json!("134386985330123457");
        *host.contexts.lock().unwrap() = vec![context(31_000), context(31_000), changed];
        assert_eq!(
            request(now).execute(&host, || Ok(now)).unwrap_err().code,
            "target_identity_changed"
        );
        assert!(host.sent.lock().unwrap().is_empty());
    }

    #[test]
    fn synthetic_submission_preserves_text_and_uses_only_existing_host_methods() {
        let now = time();
        let host = port();
        assert_eq!(
            request(now).execute(&host, || Ok(now)).unwrap().utf16_units,
            5
        );
        assert_eq!(*host.sent.lock().unwrap(), [" A😀\n"]);
        assert_eq!(
            *host.calls.lock().unwrap(),
            [
                "target.context",
                "history.get",
                "target.context",
                "restore.original",
                "history.get",
                "target.context",
                "keyboard.text"
            ]
        );
    }

    #[test]
    fn cancellation_after_restore_prevents_input() {
        let now = time();
        let request = request(now);
        let host = port();
        let mut tick = 0;
        let error = request
            .execute(&host, || {
                tick += 1;
                if tick == 8 {
                    request.cancellation.cancel();
                }
                Ok(now)
            })
            .unwrap_err();
        assert_eq!(error.code, "cancelled");
        assert!(host.sent.lock().unwrap().is_empty());
    }

    #[test]
    fn native_keyboard_refusal_and_unknown_results_are_not_retried() {
        let now = time();
        let mut host = port();
        let mut error = ActionError::new("target_changed", "不应回显的合成正文");
        error.effect_state = Some("not_started".into());
        host.keyboard = Err(error);
        let failure = request(now).execute(&host, || Ok(now)).unwrap_err();
        assert_eq!(failure.effect, InputEffect::NotStarted);
        assert!(!format!("{failure:?}").contains("合成正文"));
        assert_eq!(host.sent.lock().unwrap().len(), 1);
        host.keyboard = Ok(json!({"submitted":true,"extra":"合成正文"}));
        assert_eq!(
            request(now).execute(&host, || Ok(now)).unwrap_err().effect,
            InputEffect::Unknown
        );
    }

    #[test]
    fn expiry_or_clock_error_after_keyboard_is_unknown() {
        let now = time();
        for clock_error in [false, true] {
            let host = port();
            let mut tick = 0;
            let failure = request(now)
                .execute(&host, || {
                    tick += 1;
                    if tick == 13 {
                        if clock_error {
                            return Err(SendFailure::before("target_expired"));
                        }
                        return Ok(SendTime {
                            instant: now.instant + TARGET_LIFETIME,
                            ..now
                        });
                    }
                    Ok(now)
                })
                .unwrap_err();
            assert_eq!(failure.effect, InputEffect::Unknown);
            assert_eq!(host.sent.lock().unwrap().len(), 1);
        }
    }

    #[test]
    fn changing_selection_cancels_pending_token_and_wrong_completion_is_ignored() {
        let now = SendTime::now().unwrap();
        let target = launch(now);
        let mut session = HistorySendSession::new(Some(target), Some(456));
        assert!(session.ready_at(now));
        let request = session.begin(1, selected()).unwrap();
        assert!(session.begin(2, selected()).is_err());
        session.selection_changing("1");
        assert!(!request.cancellation.is_cancelled());
        session.selection_changing("2");
        assert!(request.cancellation.is_cancelled());
        session.finish(2);
        assert!(session.pending.is_some());
        session.finish(1);
        assert!(session.pending.is_none());
    }

    #[test]
    fn absent_or_self_window_session_has_no_sending_entry() {
        let now = time();
        assert!(!HistorySendSession::new(None, Some(456)).ready_at(now));
        assert!(!HistorySendSession::new(Some(launch(now)), None).ready_at(now));
        assert!(!HistorySendSession::new(Some(launch(now)), Some(123)).ready_at(now));
    }

    #[test]
    fn host_exported_context_is_validated_with_time_after_the_call() {
        let now = time();
        let host = port();
        *host.contexts.lock().unwrap() = vec![context(31_001)];
        let mut tick = 0;
        assert!(request(now)
            .execute(&host, || {
                tick += 1;
                Ok(if tick >= 3 {
                    SendTime {
                        instant: now.instant + Duration::from_millis(1),
                        unix_ms: 1001,
                    }
                } else {
                    now
                })
            })
            .is_ok());
        assert_eq!(host.sent.lock().unwrap().len(), 1);
    }

    #[test]
    fn expiry_immediately_before_foreground_restore_has_no_activation() {
        let now = time();
        let host = port();
        let mut tick = 0;
        let failure = request(now)
            .execute(&host, || {
                tick += 1;
                Ok(if tick == 7 {
                    SendTime {
                        instant: now.instant + TARGET_LIFETIME,
                        ..now
                    }
                } else {
                    now
                })
            })
            .unwrap_err();
        assert_eq!(failure.effect, InputEffect::NotStarted);
        assert!(!host.calls.lock().unwrap().contains(&"restore.original"));
        assert!(host.sent.lock().unwrap().is_empty());
    }

    #[test]
    fn dropping_ui_session_cancels_its_queued_request_without_host_calls() {
        let now = SendTime::now().unwrap();
        let target = launch(now);
        let mut session = HistorySendSession::new(Some(target), Some(456));
        let request = session.begin(1, selected()).unwrap();
        drop(session);
        assert!(request.cancellation.is_cancelled());
        let host = port();
        assert_eq!(
            request.execute(&host, || Ok(now)).unwrap_err().code,
            "cancelled"
        );
        assert!(host.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn trusted_target_submits_after_thirty_seconds_and_before_five_minutes() {
        let captured = time();
        for elapsed_ms in [31_000, 299_999] {
            let now = SendTime {
                instant: captured.instant + Duration::from_millis(elapsed_ms),
                unix_ms: captured.unix_ms + elapsed_ms,
            };
            // 初始DTO确已超过30秒；不能靠把CLI重放窗口一并放大通过此测试。
            assert!(crate::action_target_context::validate_value(
                context(captured.unix_ms + 30_000),
                now.unix_ms
            )
            .is_err());
            let host = port();
            *host.contexts.lock().unwrap() = vec![context(now.unix_ms + 30_000)];
            let receipt = request(captured).execute(&host, || Ok(now)).unwrap();
            assert_eq!(receipt.utf16_units, 5);
            assert_eq!(*host.sent.lock().unwrap(), [" A😀\n"]);
        }
    }

    #[test]
    fn fresh_host_context_cannot_extend_five_minute_memory_target() {
        let captured = time();
        for elapsed_ms in [300_000, 300_001] {
            let now = SendTime {
                instant: captured.instant + Duration::from_millis(elapsed_ms),
                unix_ms: captured.unix_ms + elapsed_ms,
            };
            let host = port();
            *host.contexts.lock().unwrap() = vec![context(now.unix_ms + 30_000)];
            let failure = request(captured).execute(&host, || Ok(now)).unwrap_err();
            assert_eq!(failure.code, "target_expired");
            assert_eq!(failure.effect, InputEffect::NotStarted);
            assert!(host.calls.lock().unwrap().is_empty());
            assert!(host.sent.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn live_host_context_still_obeys_untrusted_thirty_second_dto_limits() {
        let captured = time();
        let now = SendTime {
            instant: captured.instant + Duration::from_secs(31),
            unix_ms: 32_000,
        };
        assert!(launch(captured).validate(now).is_ok());
        for expires in [now.unix_ms, now.unix_ms + 30_001] {
            assert!(
                crate::action_target_context::validate_value(context(expires), now.unix_ms)
                    .is_err()
            );
            let host = port();
            *host.contexts.lock().unwrap() = vec![context(expires)];
            let failure = request(captured).execute(&host, || Ok(now)).unwrap_err();
            assert_eq!(failure.code, "target_changed");
            assert_eq!(*host.calls.lock().unwrap(), ["target.context"]);
            assert!(host.sent.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn capture_boundary_rejects_expired_or_mismatched_source_context() {
        let now = time();
        for expires in [now.unix_ms, now.unix_ms + 30_001] {
            assert!(HistoryLaunchTarget::from_captured(
                snapshot(),
                context(expires),
                99,
                now.instant,
                now
            )
            .is_err());
        }
        let other = ScriptTargetSnapshot {
            process_id: 8,
            ..snapshot()
        };
        assert_eq!(
            HistoryLaunchTarget::from_captured(other, context(31_000), 99, now.instant, now)
                .err()
                .unwrap()
                .code,
            "target_changed"
        );
        assert_eq!(
            HistoryLaunchTarget::from_captured(snapshot(), context(31_000), 7, now.instant, now)
                .err()
                .unwrap()
                .code,
            "target_is_own_window"
        );
    }

    #[test]
    fn post_thirty_second_send_keeps_cancel_identity_and_third_app_guards() {
        let captured = time();
        let now = SendTime {
            instant: captured.instant + Duration::from_secs(31),
            unix_ms: 32_000,
        };
        let attempt = request(captured);
        attempt.cancellation.cancel();
        let host = port();
        assert_eq!(
            attempt.execute(&host, || Ok(now)).unwrap_err().code,
            "cancelled"
        );
        assert!(host.calls.lock().unwrap().is_empty());
        let host = port();
        let mut changed = context(now.unix_ms + 30_000);
        changed["process_created_file_time"] = json!("134386985330123457");
        *host.contexts.lock().unwrap() = vec![changed];
        assert_eq!(
            request(captured)
                .execute(&host, || Ok(now))
                .unwrap_err()
                .code,
            "target_identity_changed"
        );
        assert!(host.sent.lock().unwrap().is_empty());
        let mut host = port();
        *host.contexts.lock().unwrap() = vec![context(now.unix_ms + 30_000)];
        host.restore = false;
        let failure = request(captured).execute(&host, || Ok(now)).unwrap_err();
        assert_eq!(failure.code, "target_changed");
        assert_eq!(failure.effect, InputEffect::NotStarted);
        assert_eq!(host.calls.lock().unwrap().last(), Some(&"restore.original"));
        assert!(host.sent.lock().unwrap().is_empty());
    }

    #[test]
    fn single_item_pending_send_has_cancel_action_until_matching_completion() {
        let now = SendTime::now().unwrap();
        let mut session = HistorySendSession::new(Some(launch(now)), Some(456));
        assert!(!session.action_enabled(false));
        let request = session.begin(1, selected()).unwrap();
        assert_eq!(session.action_label(), "取消发送");
        assert!(session.action_enabled(false));
        session.cancel();
        assert!(request.cancellation.is_cancelled());
        assert_eq!(session.action_label(), "取消中…");
        assert!(session.status().contains("等待后台确认"));
        assert!(!session.action_enabled(true));
        assert!(!session.ready());
        assert_eq!(
            session.begin(2, selected()).err().unwrap().code,
            "send_busy"
        );
        session.finish(2);
        assert!(session.pending());
        session.finish(1);
        assert!(!session.pending());
        assert_eq!(session.action_label(), "发送选中项");
        assert!(session.action_enabled(true));
    }

    #[test]
    fn expiry_during_pending_send_keeps_cancellation_confirmation_visible() {
        let now = SendTime::now().unwrap();
        let mut session = HistorySendSession::new(Some(launch(now)), Some(456));
        let request = session.begin(1, selected()).unwrap();
        session.target.as_mut().unwrap().expires = now.instant;
        assert!(session.poll());
        assert!(request.cancellation.is_cancelled());
        assert!(session.pending());
        assert_eq!(session.action_label(), "取消中…");
        assert!(session.status().contains("等待后台确认"));
        assert!(!session.status().contains("重新打开"));
        session.finish(1);
        assert!(session.status().contains("过期"));
        assert!(!session.action_enabled(true));
    }

    #[test]
    fn cancel_button_does_not_release_io_or_unknown_retry_gate_early() {
        use crate::clipboard_history_store::HistoryList;
        use crate::clipboard_history_ui_state::HistoryUiState;
        let now = SendTime::now().unwrap();
        let mut session = HistorySendSession::new(Some(launch(now)), Some(456));
        let mut state = HistoryUiState::new(true);
        state.listed(
            None,
            Ok(HistoryList {
                items: vec![selected().summary],
                revision: "2".into(),
            }),
        );
        state.select("1");
        let (id, selected) = state.begin_send_selection().unwrap();
        let request = session.begin(id, selected).unwrap();
        session.cancel();
        assert!(request.cancellation.is_cancelled());
        assert!(!state.can_query());
        assert!(state.begin_send_selection().is_none());
        session.finish(id);
        state.sent(
            id,
            Err(SendFailure {
                code: "input_result_unknown",
                effect: InputEffect::Unknown,
            }),
        );
        assert!(state.can_query());
        assert!(!session.action_enabled(state.can_send()));
        assert!(state.message.unwrap().contains("尚不确定"));
        state.listed(
            None,
            Ok(HistoryList {
                items: vec![request.selected.summary.clone()],
                revision: "2".into(),
            }),
        );
        assert!(!state.can_send());
        let refresh = state.query().unwrap();
        state.listed(
            Some(refresh),
            Ok(HistoryList {
                items: vec![request.selected.summary.clone()],
                revision: "2".into(),
            }),
        );
        assert!(session.action_enabled(state.can_send()));
    }

    #[test]
    fn confirmed_identity_failure_disarms_send_but_keeps_history_data_actions() {
        use crate::clipboard_history_store::HistoryList;
        use crate::clipboard_history_ui_state::HistoryUiState;
        for code in ["target_identity_changed", "target_is_own_window"] {
            let now = SendTime::now().unwrap();
            let mut session = HistorySendSession::new(Some(launch(now)), Some(456));
            let mut state = HistoryUiState::new(true);
            state.listed(
                None,
                Ok(HistoryList {
                    items: vec![selected().summary],
                    revision: "2".into(),
                }),
            );
            state.select("1");
            let (id, selected) = state.begin_send_selection().unwrap();
            let _request = session.begin(id, selected).unwrap();
            let result = Err(SendFailure::before(code));
            session.complete(id, &result);
            state.sent(id, result);
            assert!(!session.pending());
            assert!(session.target.is_none());
            assert!(!session.action_enabled(state.can_send()));
            assert_eq!(
                session.begin(id + 1, self::selected()).err().unwrap().code,
                "target_identity_changed"
            );
            assert!(state.can_query());
            assert!(state.can_remove());
            assert!(session.status().contains("重新打开"));
            let refresh = state.query().unwrap();
            state.listed(
                Some(refresh),
                Ok(HistoryList {
                    items: vec![self::selected().summary],
                    revision: "2".into(),
                }),
            );
            assert!(state.can_remove());
            assert!(!session.action_enabled(state.can_send()));
            assert!(session.status().contains("重新打开"));
        }
    }

    #[test]
    fn transient_foreground_failure_keeps_capability_for_an_explicit_new_attempt() {
        let now = SendTime::now().unwrap();
        let mut session = HistorySendSession::new(Some(launch(now)), Some(456));
        let original = session.target.as_ref().unwrap().snapshot;
        let _request = session.begin(1, selected()).unwrap();
        session.complete(1, &Err(SendFailure::before("target_changed")));
        assert!(!session.pending());
        assert_eq!(session.target.as_ref().unwrap().snapshot, original);
        assert!(session.action_enabled(true));
        // 只有新的明确begin产生请求；完成回执本身不提交或重试。
        assert!(session.begin(2, selected()).is_ok());
    }

    #[test]
    fn stale_identity_error_cannot_disarm_another_pending_request() {
        let now = SendTime::now().unwrap();
        let mut session = HistorySendSession::new(Some(launch(now)), Some(456));
        let _first = session.begin(1, selected()).unwrap();
        session.complete(1, &Ok(SendReceipt { utf16_units: 5 }));
        let _second = session.begin(2, selected()).unwrap();
        session.complete(1, &Err(SendFailure::before("target_identity_changed")));
        assert!(session.pending());
        assert!(session.target.is_some());
        assert!(!session.invalidated);
        assert_eq!(session.action_label(), "取消发送");
        session.complete(2, &Err(SendFailure::before("target_changed")));
        assert!(session.ready());
    }

    #[test]
    fn confirmed_snapshot_mismatch_is_distinct_from_unknown_input_and_stale_dto() {
        let now = time();
        let host = port();
        let mut changed = context(31_000);
        changed["process_id"] = json!(8);
        *host.contexts.lock().unwrap() = vec![changed];
        let failure = request(now).execute(&host, || Ok(now)).unwrap_err();
        assert_eq!(failure.code, "target_identity_changed");
        assert!(failure.invalidates_target());
        assert_eq!(*host.calls.lock().unwrap(), ["target.context"]);
        assert!(host.sent.lock().unwrap().is_empty());
        assert!(!(SendFailure {
            code: "input_result_unknown",
            effect: InputEffect::Unknown
        })
        .invalidates_target());
        let host = port();
        *host.contexts.lock().unwrap() = vec![context(now.unix_ms)];
        let failure = request(now).execute(&host, || Ok(now)).unwrap_err();
        assert_eq!(failure.code, "target_changed");
        assert!(!failure.invalidates_target());
        struct ProbeFailure {
            code: &'static str,
        }
        impl NativeHost for ProbeFailure {
            fn call(
                &self,
                method: &str,
                _: Value,
                _: &CancellationToken,
            ) -> Result<Value, ActionError> {
                assert_eq!(method, "target.context");
                let mut error = ActionError::new(self.code, "合成元数据错误，不应进入界面");
                error.effect_state = Some("not_started".into());
                Err(error)
            }
        }
        impl SendPort for ProbeFailure {
            fn restore_original(
                &self,
                _: isize,
                _: &CancellationToken,
                _: Instant,
            ) -> Result<(), SendFailure> {
                panic!("身份查询失败后不应交还前台")
            }
        }
        for (code, invalidates) in [("target_changed", true), ("target_unavailable", false)] {
            let failure = request(now)
                .execute(&ProbeFailure { code }, || Ok(now))
                .unwrap_err();
            assert_eq!(failure.invalidates_target(), invalidates);
            assert!(!format!("{failure:?}").contains("合成元数据错误"));
        }
    }
}

#[cfg(test)]
#[path = "clipboard_history_send_regression.rs"]
mod regression;
