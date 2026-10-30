//! 浏览器动作的应用层模型与有界后台执行。原生客户端连接由后台线程持有。
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
        Arc, Mutex,
    },
    thread,
};

const CAPACITY: usize = 4;
const MAX_ACTION_BYTES: usize = 240 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserAction {
    #[serde(deserialize_with = "deserialize_command")]
    pub command: BrowserCommand,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

fn deserialize_command<'de, D>(deserializer: D) -> Result<BrowserCommand, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let value = Value::deserialize(deserializer)?;
    let fields = value
        .as_object()
        .ok_or_else(|| D::Error::custom("浏览器动作需要对象格式。"))?;
    // Serde 的内部标签无参变体会忽略额外字段，在保存动作前显式拒绝，保留输入语义。
    let allowed: &[&str] = match fields.get("operation").and_then(Value::as_str) {
        Some("new_tab") => &["operation", "url"],
        Some("call") => &["operation", "method", "args"],
        _ => &["operation"],
    };
    if let Some(unknown) = fields.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(D::Error::custom(format!(
            "浏览器动作包含未知字段：{unknown}"
        )));
    }
    BrowserCommand::deserialize(value).map_err(D::Error::custom)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserCommand {
    NewTab {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
    },
    CloseTab,
    ReloadTab,
    DuplicateTab,
    TogglePin,
    ToggleMute,
    NextTab,
    PreviousTab,
    GoBack,
    GoForward,
    RestoreTab,
    NewWindow,
    Call {
        method: String,
        #[serde(default)]
        args: Vec<Value>,
    },
}

impl BrowserCommand {
    pub fn label(&self) -> &'static str {
        match self {
            Self::NewTab { .. } => "新建标签页",
            Self::CloseTab => "关闭当前标签页",
            Self::ReloadTab => "刷新当前标签页",
            Self::DuplicateTab => "复制当前标签页",
            Self::TogglePin => "切换标签页固定",
            Self::ToggleMute => "切换标签页静音",
            Self::NextTab => "下一个标签页",
            Self::PreviousTab => "上一个标签页",
            Self::GoBack => "网页后退",
            Self::GoForward => "网页前进",
            Self::RestoreTab => "恢复最近关闭",
            Self::NewWindow => "新建浏览器窗口",
            Self::Call { .. } => "调用 Chrome API",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::NewTab { .. } => "在 Chrome 中新建标签页，可指定要打开的网址。",
            Self::CloseTab => "关闭 Chrome 最近使用窗口中的活动标签页。",
            Self::ReloadTab => "重新载入 Chrome 最近使用窗口中的活动标签页。",
            Self::DuplicateTab => "复制当前网页及其标签页。",
            Self::TogglePin => "固定当前标签页，再次触发取消固定。",
            Self::ToggleMute => "静音当前标签页，再次触发恢复声音。",
            Self::NextTab => "在当前 Chrome 窗口中向右切换，到末尾后回到开头。",
            Self::PreviousTab => "在当前 Chrome 窗口中向左切换，到开头后回到末尾。",
            Self::GoBack => "在当前标签页的浏览历史中后退。",
            Self::GoForward => "在当前标签页的浏览历史中前进。",
            Self::RestoreTab => "恢复 Chrome 最近关闭的标签页或窗口，需要会话权限。",
            Self::NewWindow => "新建普通 Chrome 窗口。",
            Self::Call { .. } => "按方法名与 JSON 参数直接调用扩展支持的 chrome.* API。",
        }
    }
}

impl BrowserAction {
    pub fn label(&self) -> &'static str {
        self.command.label()
    }

    pub fn description(&self) -> &'static str {
        self.command.description()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.profile.as_ref().is_some_and(|profile| {
            profile.trim().is_empty()
                || profile.len() > 128
                || profile.chars().any(char::is_control)
        }) {
            return Err("浏览器配置标识需要 1–128 字节，不能包含控制字符。".into());
        }
        match &self.command {
            BrowserCommand::NewTab { url: Some(url) }
                if url.trim().is_empty() || url.contains('\0') || url.len() > 64 * 1024 =>
            {
                return Err("网址不能为空、包含空字符或超过 64 KiB。".into());
            }
            BrowserCommand::Call { method, .. } if !valid_method(method) => {
                return Err(
                    "Chrome API 方法应为 bookmarks.getTree 这样的路径，不能带 chrome. 前缀。"
                        .into(),
                );
            }
            _ => {}
        }
        if serde_json::to_vec(self)
            .map_err(|error| error.to_string())?
            .len()
            > MAX_ACTION_BYTES
        {
            return Err("浏览器动作参数不能超过 240 KiB。".into());
        }
        Ok(())
    }
}

fn valid_method(method: &str) -> bool {
    if method.len() > 160 || method.starts_with("chrome.") {
        return false;
    }
    let pieces: Vec<_> = method.split('.').collect();
    (2..=4).contains(&pieces.len())
        && pieces.iter().all(|piece| {
            !piece.is_empty()
                && !matches!(*piece, "__proto__" | "constructor" | "prototype")
                && piece.as_bytes()[0].is_ascii_alphabetic()
                && piece
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
}

#[derive(Debug, PartialEq, Eq)]
pub struct BrowserEvent {
    pub id: u64,
    pub outcome: BrowserOutcome,
}

#[derive(Debug, PartialEq, Eq)]
pub enum BrowserOutcome {
    Started,
    Succeeded {
        output: Value,
    },
    Failed {
        code: String,
        message: String,
        effect_state: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Fault {
    code: String,
    message: String,
    effect_state: Option<String>,
}

impl Fault {
    fn before(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            effect_state: Some("not_started".into()),
        }
    }
}

struct Request {
    id: u64,
    action: BrowserAction,
}

pub struct BrowserExecutor {
    requests: Option<SyncSender<Request>>,
    events: Receiver<BrowserEvent>,
    reserved: Mutex<HashSet<u64>>,
    stopping: Arc<AtomicBool>,
    startup_error: Option<String>,
}

impl BrowserExecutor {
    /// 构造只创建线程和队列，不在界面线程读取插件配置或连接浏览器。
    pub fn new(config: PathBuf) -> Self {
        let (requests, receiver) = mpsc::sync_channel::<Request>(CAPACITY);
        let (sender, events) = mpsc::sync_channel(CAPACITY * 2);
        let stopping = Arc::new(AtomicBool::new(false));
        let worker_stopping = stopping.clone();
        let spawned = thread::Builder::new()
            .name("omnitrigger-browser-actions".into())
            .spawn(move || {
                struct Stop(Arc<AtomicBool>);
                impl Drop for Stop {
                    fn drop(&mut self) {
                        self.0.store(true, Ordering::Release);
                    }
                }
                let _stop = Stop(worker_stopping.clone());
                worker(config, receiver, sender, &worker_stopping);
            });
        let startup_error = spawned
            .err()
            .map(|error| format!("浏览器动作线程启动失败：{error}"));
        Self {
            requests: Some(requests),
            events,
            reserved: Mutex::new(HashSet::new()),
            stopping,
            startup_error,
        }
    }

    pub fn submit(&self, id: u64, action: BrowserAction) -> Result<(), String> {
        action.validate()?;
        if let Some(error) = &self.startup_error {
            return Err(error.clone());
        }
        if self.stopping.load(Ordering::Acquire) {
            return Err("浏览器动作服务已停止。".into());
        }
        let mut reserved = self.reserved.lock().map_err(|_| "浏览器动作状态不可用。")?;
        if reserved.len() >= CAPACITY {
            return Err("浏览器动作队列已满，请稍后重试。".into());
        }
        if !reserved.insert(id) {
            return Err("此浏览器动作尚未完成。".into());
        }
        let result = self
            .requests
            .as_ref()
            .ok_or("浏览器动作服务已停止。")?
            .try_send(Request { id, action })
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => "浏览器动作队列已满，请稍后重试。".to_string(),
                mpsc::TrySendError::Disconnected(_) => "浏览器动作服务已停止。".to_string(),
            });
        if result.is_err() {
            reserved.remove(&id);
        }
        result
    }

    pub fn is_running(&self, id: u64) -> bool {
        self.reserved
            .lock()
            .map(|entries| entries.contains(&id))
            .unwrap_or(true)
    }

    pub fn try_recv(&self) -> Result<BrowserEvent, TryRecvError> {
        let event = self.events.try_recv()?;
        if !matches!(event.outcome, BrowserOutcome::Started) {
            if let Ok(mut reserved) = self.reserved.lock() {
                reserved.remove(&event.id);
            }
        }
        Ok(event)
    }
}

impl Drop for BrowserExecutor {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        self.requests.take();
        // 不在界面线程 join 网络等待；当前调用结束后后台自行释放连接。
    }
}

fn worker(
    config: PathBuf,
    receiver: Receiver<Request>,
    sender: SyncSender<BrowserEvent>,
    stopping: &AtomicBool,
) {
    #[cfg(windows)]
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();
    #[cfg(windows)]
    let mut transport = NativeTransport::default();
    while let Ok(request) = receiver.recv() {
        if stopping.load(Ordering::Acquire) {
            break;
        }
        if sender
            .send(BrowserEvent {
                id: request.id,
                outcome: BrowserOutcome::Started,
            })
            .is_err()
        {
            break;
        }
        #[cfg(windows)]
        let result = match &runtime {
            Ok(runtime) => runtime.block_on(async {
                let endpoint = connector_endpoint(&config)?;
                transport.set_endpoint(endpoint);
                execute(&mut transport, &request.action).await
            }),
            Err(error) => Err(Fault::before(
                "runtime_unavailable",
                format!("浏览器运行时启动失败：{error}"),
            )),
        };
        #[cfg(not(windows))]
        let result: Result<Value, Fault> = {
            let _ = (&config, &request.action);
            Err(Fault::before(
                "unsupported_platform",
                "浏览器连接器目前仅支持 Windows。",
            ))
        };
        let outcome = match result {
            Ok(output) => BrowserOutcome::Succeeded { output },
            Err(Fault {
                code,
                message,
                effect_state,
            }) => BrowserOutcome::Failed {
                code,
                message,
                effect_state,
            },
        };
        if sender
            .send(BrowserEvent {
                id: request.id,
                outcome,
            })
            .is_err()
        {
            break;
        }
    }
}

#[cfg(windows)]
fn connector_endpoint(config: &Path) -> Result<String, Fault> {
    use std::io::Read;
    let snapshot = crate::browser_connector::load(config)
        .map_err(|error| Fault::before(&error.code, error.message))?;
    if !snapshot.installed || !snapshot.registered || !snapshot.enabled {
        return Err(Fault::before(
            "connector_unavailable",
            "请先在“扩展”页安装并启用浏览器连接器。",
        ));
    }
    let path = snapshot.cli_path.with_file_name("connection.json");
    crate::browser_connector::ensure_plain(&path)
        .map_err(|error| Fault::before(&error.code, error.message))?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(65537).read_to_end(&mut bytes))
        .map_err(|error| {
            Fault::before(
                "connection_config",
                format!("无法读取浏览器连接配置：{error}"),
            )
        })?;
    if bytes.len() > 65536 {
        return Err(Fault::before(
            "connection_config",
            "浏览器连接配置超过大小限制。",
        ));
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|error| Fault::before("connection_config", error.to_string()))?;
    let pipe = value["pipe"]
        .as_str()
        .ok_or_else(|| Fault::before("connection_config", "浏览器连接配置缺少本机端点。"))?;
    if !pipe.starts_with(r"\\.\pipe\")
        || pipe.len() <= 9
        || pipe.len() > 240
        || pipe.contains('\0')
        || pipe[9..].contains(['\\', '/'])
    {
        return Err(Fault::before(
            "connection_config",
            "浏览器连接配置不是有效的本机 Named Pipe。",
        ));
    }
    Ok(pipe.into())
}

trait Transport {
    async fn profiles(&mut self) -> Result<Value, Fault>;
    async fn call(&mut self, profile: &str, method: &str, args: Value) -> Result<Value, Fault>;
}

#[cfg(windows)]
#[derive(Default)]
struct NativeTransport {
    endpoint: String,
    discovery: Option<omnitrigger_browser::client::BrowserClient>,
    clients: std::collections::HashMap<String, omnitrigger_browser::client::BrowserClient>,
}

#[cfg(windows)]
impl NativeTransport {
    fn set_endpoint(&mut self, endpoint: String) {
        if self.endpoint != endpoint {
            self.endpoint = endpoint;
            self.discovery = None;
            self.clients.clear();
        }
    }
}

#[cfg(windows)]
impl From<omnitrigger_browser::client::RpcError> for Fault {
    fn from(error: omnitrigger_browser::client::RpcError) -> Self {
        let message = if error.code == "transport_error" {
            format!(
                "无法连接浏览器，请确认 Chrome 和浏览器连接器已运行：{}",
                error.message
            )
        } else {
            error.message
        };
        Self {
            code: error.code,
            message,
            effect_state: error.effect_state,
        }
    }
}

#[cfg(windows)]
impl Transport for NativeTransport {
    async fn profiles(&mut self) -> Result<Value, Fault> {
        if self.discovery.is_none() {
            self.discovery = Some(
                omnitrigger_browser::client::BrowserClient::connect(&self.endpoint, None).await?,
            );
        }
        let result = self
            .discovery
            .as_mut()
            .expect("discovery initialized")
            .profiles()
            .await;
        if result.is_err() {
            self.discovery = None;
        }
        result.map_err(Into::into)
    }

    async fn call(&mut self, profile: &str, method: &str, args: Value) -> Result<Value, Fault> {
        if !self.clients.contains_key(profile) {
            // 不重试：连接失败属于本次动作的终态，下一次用户触发才可重新建立。
            let client = omnitrigger_browser::client::BrowserClient::connect(
                &self.endpoint,
                Some(profile.into()),
            )
            .await?;
            // 只缓存最近使用的一个 profile，反复切换也不累积后台连接。
            self.clients.clear();
            self.clients.insert(profile.into(), client);
        }
        let result = self
            .clients
            .get_mut(profile)
            .expect("profile initialized")
            .call(method, args, 10000)
            .await;
        if result.is_err() {
            self.clients.remove(profile);
        }
        result.map_err(Into::into)
    }
}

fn select_profile(profiles: Value, requested: Option<&str>) -> Result<String, Fault> {
    let profiles = profiles
        .as_array()
        .ok_or_else(|| Fault::before("invalid_response", "浏览器配置列表格式错误。"))?;
    let mut ids = HashSet::new();
    for profile in profiles {
        let id = profile["browser_instance_id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 128)
            .ok_or_else(|| Fault::before("invalid_response", "浏览器配置缺少有效标识。"))?;
        if !ids.insert(id) {
            return Err(Fault::before("invalid_response", "浏览器配置标识重复。"));
        }
    }
    if let Some(requested) = requested {
        return if ids.contains(requested) {
            Ok(requested.into())
        } else {
            Err(Fault::before(
                "profile_unavailable",
                "指定的 Chrome 配置尚未连接。",
            ))
        };
    }
    match ids.len() {
        0 => Err(Fault::before(
            "browser_unavailable",
            "Chrome 尚未连接，请打开浏览器并启用 OmniTrigger 扩展。",
        )),
        1 => Ok(ids.into_iter().next().expect("one profile").into()),
        _ => Err(Fault::before(
            "ambiguous_profile",
            "有多个 Chrome 配置已连接，请为动作选择明确的浏览器配置。",
        )),
    }
}

async fn active_tab(transport: &mut impl Transport, profile: &str) -> Result<Value, Fault> {
    let value = transport
        .call(
            profile,
            "tabs.query",
            json!([{"active":true,"lastFocusedWindow":true}]),
        )
        .await?;
    let tabs = value
        .as_array()
        .ok_or_else(|| Fault::before("invalid_response", "标签页查询结果格式错误。"))?;
    if tabs.len() != 1 {
        return Err(Fault::before(
            "active_tab_unavailable",
            "无法唯一确定 Chrome 当前标签页。",
        ));
    }
    tab_id(&tabs[0])?;
    window_id(&tabs[0])?;
    Ok(tabs[0].clone())
}

fn nonnegative_id(value: &Value, field: &str) -> Result<i64, Fault> {
    value[field]
        .as_i64()
        .filter(|id| *id >= 0)
        .ok_or_else(|| Fault::before("invalid_response", format!("标签页缺少有效 {field}。")))
}
fn tab_id(tab: &Value) -> Result<i64, Fault> {
    nonnegative_id(tab, "id")
}
fn window_id(tab: &Value) -> Result<i64, Fault> {
    nonnegative_id(tab, "windowId")
}

async fn execute(transport: &mut impl Transport, action: &BrowserAction) -> Result<Value, Fault> {
    action
        .validate()
        .map_err(|message| Fault::before("invalid_action", message))?;
    let profile = select_profile(transport.profiles().await?, action.profile.as_deref())?;
    match &action.command {
        BrowserCommand::NewTab { url } => {
            let properties = url
                .as_ref()
                .map(|url| json!({"url":url}))
                .unwrap_or_else(|| json!({}));
            transport
                .call(&profile, "tabs.create", json!([properties]))
                .await
        }
        BrowserCommand::NewWindow => {
            transport
                .call(&profile, "windows.create", json!([{}]))
                .await
        }
        BrowserCommand::RestoreTab => {
            transport
                .call(&profile, "sessions.restore", json!([]))
                .await
        }
        BrowserCommand::Call { method, args } => {
            transport
                .call(&profile, method, Value::Array(args.clone()))
                .await
        }
        command => {
            let tab = active_tab(transport, &profile).await?;
            let id = tab_id(&tab)?;
            match command {
                BrowserCommand::CloseTab => {
                    transport.call(&profile, "tabs.remove", json!([id])).await
                }
                BrowserCommand::ReloadTab => {
                    transport.call(&profile, "tabs.reload", json!([id])).await
                }
                BrowserCommand::DuplicateTab => {
                    transport
                        .call(&profile, "tabs.duplicate", json!([id]))
                        .await
                }
                BrowserCommand::GoBack => {
                    transport.call(&profile, "tabs.goBack", json!([id])).await
                }
                BrowserCommand::GoForward => {
                    transport
                        .call(&profile, "tabs.goForward", json!([id]))
                        .await
                }
                BrowserCommand::TogglePin => {
                    let pinned = tab["pinned"]
                        .as_bool()
                        .ok_or_else(|| Fault::before("invalid_response", "标签页缺少固定状态。"))?;
                    transport
                        .call(&profile, "tabs.update", json!([id,{"pinned":!pinned}]))
                        .await
                }
                BrowserCommand::ToggleMute => {
                    let muted = tab["mutedInfo"]["muted"]
                        .as_bool()
                        .ok_or_else(|| Fault::before("invalid_response", "标签页缺少静音状态。"))?;
                    transport
                        .call(&profile, "tabs.update", json!([id,{"muted":!muted}]))
                        .await
                }
                BrowserCommand::NextTab | BrowserCommand::PreviousTab => {
                    let window = window_id(&tab)?;
                    let value = transport
                        .call(&profile, "tabs.query", json!([{"windowId":window}]))
                        .await?;
                    let tabs = value
                        .as_array()
                        .ok_or_else(|| Fault::before("invalid_response", "标签页列表格式错误。"))?;
                    let mut ordered = Vec::new();
                    let mut indices = HashSet::new();
                    let mut ids = HashSet::new();
                    for entry in tabs {
                        let entry_id = tab_id(entry)?;
                        let index = nonnegative_id(entry, "index")?;
                        if window_id(entry)? != window
                            || !indices.insert(index)
                            || !ids.insert(entry_id)
                        {
                            return Err(Fault::before(
                                "invalid_response",
                                "标签页列表包含不同窗口或重复位置。",
                            ));
                        }
                        ordered.push((index, entry_id, entry));
                    }
                    ordered.sort_unstable_by_key(|entry| entry.0);
                    let current =
                        ordered
                            .iter()
                            .position(|entry| entry.1 == id)
                            .ok_or_else(|| {
                                Fault::before(
                                    "active_tab_unavailable",
                                    "当前标签页已离开原窗口，未执行切换。",
                                )
                            })?;
                    if ordered.len() == 1 {
                        return Ok(tab);
                    }
                    let next = if matches!(command, BrowserCommand::NextTab) {
                        (current + 1) % ordered.len()
                    } else {
                        (current + ordered.len() - 1) % ordered.len()
                    };
                    let result = transport
                        .call(
                            &profile,
                            "tabs.update",
                            json!([ordered[next].1,{"active":true}]),
                        )
                        .await?;
                    // 第二步失败时不会重复激活；Chrome 已完成的第一步可能仍有效。
                    transport
                        .call(&profile, "windows.update", json!([window,{"focused":true}]))
                        .await
                        .map_err(|mut error| {
                            error.effect_state = Some("unknown".into());
                            error
                        })?;
                    Ok(result)
                }
                _ => unreachable!("non-tab commands handled above"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct Fake {
        profiles: Value,
        replies: VecDeque<Result<Value, Fault>>,
        calls: Vec<(String, String, Value)>,
    }
    impl Fake {
        fn new(replies: Vec<Value>) -> Self {
            Self {
                profiles: json!([{"browser_instance_id":"profile-a"}]),
                replies: replies.into_iter().map(Ok).collect(),
                calls: vec![],
            }
        }
    }
    impl Transport for Fake {
        async fn profiles(&mut self) -> Result<Value, Fault> {
            Ok(self.profiles.clone())
        }
        async fn call(&mut self, profile: &str, method: &str, args: Value) -> Result<Value, Fault> {
            self.calls.push((profile.into(), method.into(), args));
            self.replies.pop_front().expect("unexpected API call")
        }
    }
    fn run(fake: &mut Fake, command: BrowserCommand) -> Result<Value, Fault> {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(execute(
                fake,
                &BrowserAction {
                    command,
                    profile: None,
                },
            ))
    }
    fn tab() -> Value {
        json!({"id":7,"windowId":3,"index":2,"pinned":false,"mutedInfo":{"muted":true}})
    }

    #[test]
    fn default_new_tab_omits_url_and_uses_a_fixed_profile() {
        let mut fake = Fake::new(vec![json!({"id":8})]);
        run(&mut fake, BrowserCommand::NewTab { url: None }).unwrap();
        assert_eq!(
            fake.calls,
            vec![("profile-a".into(), "tabs.create".into(), json!([{}]))]
        );
    }
    #[test]
    fn ambiguous_profiles_do_not_dispatch_any_api() {
        let mut fake = Fake::new(vec![]);
        fake.profiles = json!([{"browser_instance_id":"a"},{"browser_instance_id":"b"}]);
        assert_eq!(
            run(&mut fake, BrowserCommand::CloseTab).unwrap_err().code,
            "ambiguous_profile"
        );
        assert!(fake.calls.is_empty());
        assert_eq!(select_profile(fake.profiles, Some("b")).unwrap(), "b");
        assert_eq!(
            select_profile(json!([]), None).unwrap_err().code,
            "browser_unavailable"
        );
    }
    #[test]
    fn active_tab_must_be_unique_before_a_mutation() {
        for tabs in [
            json!([]),
            json!([tab(), tab()]),
            json!([{"id":-1,"windowId":3}]),
        ] {
            let mut fake = Fake::new(vec![tabs]);
            assert!(run(&mut fake, BrowserCommand::CloseTab).is_err());
            assert_eq!(fake.calls.len(), 1);
            assert_eq!(
                fake.calls[0].2,
                json!([{"active":true,"lastFocusedWindow":true}])
            );
        }
    }
    #[test]
    fn pin_and_mute_toggle_the_observed_tab_with_explicit_id() {
        for (command, properties) in [
            (BrowserCommand::TogglePin, json!({"pinned":true})),
            (BrowserCommand::ToggleMute, json!({"muted":false})),
        ] {
            let mut fake = Fake::new(vec![json!([tab()]), json!({})]);
            run(&mut fake, command).unwrap();
            assert_eq!(
                fake.calls[1],
                (
                    "profile-a".into(),
                    "tabs.update".into(),
                    json!([7, properties])
                )
            );
        }
    }
    #[test]
    fn a_failed_side_effect_is_never_retried() {
        let mut fake = Fake::new(vec![json!([tab()])]);
        let failure = Fault {
            code: "result_unknown".into(),
            message: "断线".into(),
            effect_state: Some("unknown".into()),
        };
        fake.replies.push_back(Err(failure.clone()));
        assert_eq!(
            run(&mut fake, BrowserCommand::CloseTab).unwrap_err(),
            failure
        );
        assert_eq!(fake.calls.len(), 2);
        assert_eq!(fake.calls[1].1, "tabs.remove");
    }
    #[test]
    fn adjacent_tabs_are_sorted_by_index_and_wrap_in_the_same_window() {
        let other = json!({"id":9,"windowId":3,"index":0});
        for command in [BrowserCommand::NextTab, BrowserCommand::PreviousTab] {
            let mut fake = Fake::new(vec![
                json!([tab()]),
                json!([tab(), other]),
                json!({"id":9}),
                json!({}),
            ]);
            run(&mut fake, command).unwrap();
            assert!(fake.calls.iter().all(|entry| entry.0 == "profile-a"));
            assert_eq!(fake.calls[1].2, json!([{"windowId":3}]));
            assert_eq!(fake.calls[2].2, json!([9,{"active":true}]));
            assert_eq!(fake.calls[3].2, json!([3,{"focused":true}]));
        }
    }
    #[test]
    fn no_tab_switch_when_snapshot_is_inconsistent_or_only_one_tab_exists() {
        let mut single = Fake::new(vec![json!([tab()]), json!([tab()])]);
        run(&mut single, BrowserCommand::NextTab).unwrap();
        assert_eq!(single.calls.len(), 2);
        for tabs in [
            json!([]),
            json!([{"id":7,"windowId":4,"index":0}]),
            json!([tab(), tab()]),
        ] {
            let mut fake = Fake::new(vec![json!([tab()]), tabs]);
            assert!(run(&mut fake, BrowserCommand::NextTab).is_err());
            assert_eq!(fake.calls.len(), 2);
        }
    }
    #[test]
    fn generic_api_validation_rejects_unsafe_paths_and_oversize_arguments() {
        for method in [
            "chrome.tabs.query",
            "tabs.__proto__.x",
            "runtime.constructor",
            "tabs[0].x",
            "tabs",
        ] {
            assert!(BrowserAction {
                command: BrowserCommand::Call {
                    method: method.into(),
                    args: vec![]
                },
                profile: None
            }
            .validate()
            .is_err());
        }
        assert!(BrowserAction {
            command: BrowserCommand::Call {
                method: "bookmarks.search".into(),
                args: vec![json!("x".repeat(MAX_ACTION_BYTES))]
            },
            profile: None
        }
        .validate()
        .is_err());
        let action = BrowserAction {
            command: BrowserCommand::Call {
                method: "bookmarks.getTree".into(),
                args: vec![],
            },
            profile: Some("profile-a".into()),
        };
        action.validate().unwrap();
        assert_eq!(
            serde_json::from_value::<BrowserAction>(serde_json::to_value(&action).unwrap())
                .unwrap(),
            action
        );
        assert!(serde_json::from_value::<BrowserAction>(
            json!({"command":{"operation":"reload_tab","unexpected":1}})
        )
        .is_err());
    }

    /// 真实 Windows IPC / 安装配置 / Rust 客户端集成；扩展对端是模拟端，并非真实 Chrome。
    #[cfg(windows)]
    #[test]
    fn windows_named_pipe_executor_reassembles_a_large_unicode_bookmark_tree() {
        use omnitrigger_browser::{broker, framing, platform, CHUNK_BYTES, MAX_FRAME};
        use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

        struct Fixture {
            root: PathBuf,
            parent: PathBuf,
        }
        impl Drop for Fixture {
            fn drop(&mut self) {
                // 仅清理本测试明确创建的临时目录，先验证绝对路径和最终解析后的父目录。
                if self.root.is_absolute()
                    && self.root.parent() == Some(self.parent.as_path())
                    && self.root.file_name().is_some_and(|name| {
                        name.to_string_lossy()
                            .starts_with("omnitrigger-browser-app-test-")
                    })
                    && std::fs::canonicalize(&self.root)
                        .ok()
                        .and_then(|root| root.parent().map(Path::to_path_buf))
                        == std::fs::canonicalize(&self.parent).ok()
                {
                    let _ = std::fs::remove_dir_all(&self.root);
                }
            }
        }
        fn base64(bytes: &[u8]) -> String {
            const ALPHABET: &[u8] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
            for chunk in bytes.chunks(3) {
                let first = chunk[0] as usize;
                let second = chunk.get(1).copied().unwrap_or(0) as usize;
                let third = chunk.get(2).copied().unwrap_or(0) as usize;
                encoded.push(ALPHABET[first >> 2] as char);
                encoded.push(ALPHABET[((first & 3) << 4) | (second >> 4)] as char);
                encoded.push(if chunk.len() > 1 {
                    ALPHABET[((second & 15) << 2) | (third >> 6)] as char
                } else {
                    '='
                });
                encoded.push(if chunk.len() > 2 {
                    ALPHABET[third & 63] as char
                } else {
                    '='
                });
            }
            encoded
        }

        let nonce = format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let parent = std::env::temp_dir();
        let fixture = Fixture {
            root: parent.join(format!("omnitrigger-browser-app-test-{nonce}")),
            parent,
        };
        let config = fixture.root.join("bindings.json");
        let runtime = crate::browser_connector::runtime_root(&config)
            .join(crate::browser_connector::CONNECTOR_VERSION);
        std::fs::create_dir_all(&runtime).unwrap();
        let cli_path = runtime.join("omnitrigger-browser.exe");
        std::fs::write(&cli_path, []).unwrap();
        std::fs::write(runtime.join("omnitrigger-browser-host.exe"), []).unwrap();
        let snapshot = crate::browser_connector::Snapshot {
            enabled: true,
            registered: true,
            installed: true,
            extension_id: "abcdefghijklmnopabcdefghijklmnop".into(),
            version: crate::browser_connector::CONNECTOR_VERSION.into(),
            cli_path,
            ..Default::default()
        };
        std::fs::write(
            crate::browser_connector::state_path(&config),
            serde_json::to_vec(&snapshot).unwrap(),
        )
        .unwrap();
        let pipe = format!(r"\\.\pipe\omnitrigger-browser-app-test-{nonce}");
        std::fs::write(
            runtime.join("connection.json"),
            serde_json::to_vec(&json!({"pipe":pipe,"extension_ids":[snapshot.extension_id]}))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(connector_endpoint(&config).unwrap(), pipe);

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let bridge_pipe = pipe.clone();
            let bridge = tokio::spawn(async move { broker::serve(&bridge_pipe).await });
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut extension = loop {
                match platform::connect(&pipe).await {
                    Ok(extension) => break extension,
                    Err(error) if Instant::now() < deadline && error.raw_os_error() == Some(2) => {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                    Err(error) => panic!("隔离测试桥接启动失败：{error}"),
                }
            };
            framing::write(&mut extension, &json!({
                "v":1,"type":"hello","role":"extension","name":"测试浏览器",
                "browser_instance_id":"profile-fixture","session_id":"session-fixture",
                "capabilities":{"methods":["bookmarks.getTree"]}
            })).await.unwrap();
            assert_eq!(framing::read(&mut extension).await.unwrap()["ok"], true);
            let expected = json!([{"id":"0","title":"书签根目录","children":(0..2500).map(|index| {
                json!({"id":index.to_string(),"title":"中文书签🙂🦀".repeat(20),"url":format!("https://example.test/{index}")})
            }).collect::<Vec<_>>()}]);
            let bytes = serde_json::to_vec(&expected).unwrap();
            assert!(bytes.len() > MAX_FRAME * 2);
            let chunk_count = bytes.len().div_ceil(CHUNK_BYTES);
            let fake_extension = tokio::spawn(async move {
                let request = framing::read(&mut extension).await.unwrap();
                assert_eq!(request["method"], "bookmarks.getTree");
                assert_eq!(request["args"], json!([]));
                assert_eq!(request["browser_instance_id"], "profile-fixture");
                assert_eq!(request["session_id"], "session-fixture");
                let id = request["id"].clone();
                assert!(id.as_str().unwrap().starts_with("bridge:"));
                framing::write(&mut extension, &json!({
                    "v":1,"type":"result.begin","id":id,"session_id":"session-fixture",
                    "total_bytes":bytes.len(),"chunk_count":chunk_count
                })).await.unwrap();
                for (index, chunk) in bytes.chunks(CHUNK_BYTES).enumerate() {
                    let message = json!({"v":1,"type":"result.chunk","id":id,"session_id":"session-fixture","index":index,"data":base64(chunk)});
                    assert!(framing::encode(&message).unwrap().len() <= MAX_FRAME + 4);
                    framing::write(&mut extension, &message).await.unwrap();
                    let acknowledgement = framing::read(&mut extension).await.unwrap();
                    assert_eq!(acknowledgement["type"], "result.ack");
                    assert_eq!(acknowledgement["id"], id);
                    assert_eq!(acknowledgement["index"], index);
                }
                framing::write(&mut extension, &json!({"v":1,"type":"result.end","id":id,"session_id":"session-fixture"})).await.unwrap();
                chunk_count
            });
            let executor = BrowserExecutor::new(config);
            executor.submit(41, BrowserAction {
                command: BrowserCommand::Call { method:"bookmarks.getTree".into(),args:vec![] },
                profile:None,
            }).unwrap();
            let result = tokio::time::timeout(Duration::from_secs(15), async {
                let mut started = false;
                loop {
                    match executor.try_recv() {
                        Ok(BrowserEvent { id, outcome:BrowserOutcome::Started }) => {
                            assert_eq!(id,41);
                            assert!(!started);
                            started = true;
                            assert!(executor.is_running(41));
                        }
                        Ok(BrowserEvent { id, outcome:BrowserOutcome::Succeeded { output } }) => {
                            assert_eq!(id,41);
                            assert!(started);
                            assert!(!executor.is_running(41));
                            break output;
                        }
                        Ok(event) => panic!("浏览器动作集成失败：{event:?}"),
                        Err(TryRecvError::Empty) => tokio::time::sleep(Duration::from_millis(2)).await,
                        Err(error) => panic!("浏览器动作事件通道中断：{error}"),
                    }
                }
            }).await.expect("测试浏览器动作超时");
            assert_eq!(result, expected);
            assert_eq!(fake_extension.await.unwrap(), chunk_count);
            drop(executor);
            bridge.abort();
            let _ = bridge.await;
        });
    }
}
