//! 每次调用独立的本机能力桥接。只监听回环地址，不使用脚本 stdout。
use crate::{ActionError, CancellationToken};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

// 专用类名拒绝是上层静默无操作的证明；必须先于通用读错误正规化验证原字段。
fn preserve_class_rejection(method: &str, error: &mut ActionError) -> bool {
    if error.code != "window_class_blocked" {
        return false;
    }
    if !matches!(
        method,
        "window.geometry" | "window.set_rect" | "window.drag"
    ) || error.method.as_deref() != Some(method)
        || !matches!(
            error.effect_state.as_deref(),
            Some("not_started" | "unknown")
        )
    {
        error.code = "invalid_response".into();
        error.method = Some(method.into());
        error.effect_state = Some("unknown".into());
    }
    true
}

// 65,536 个 UTF-16 单元即使全部需要 JSON 转义也能完整传输。
const LIMIT: usize = 512 * 1024;
const RESPONSE_LIMIT: usize = 512 * 1024;
const BROWSER_RESPONSE_LIMIT: usize = 32 * 1024 * 1024 + 1024;
const CLIENTS: usize = 4;
const METHODS: [&str; 52] = [
    "mouse.left_down",
    "mouse.left_up",
    "actions.stop_other_instances",
    "state.read",
    "state.compare_and_swap",
    "system.open",
    "system.open_file_in_parent",
    "process.capture",
    "history.list",
    "history.get",
    "history.append",
    "history.remove",
    "fs.stat",
    "fs.read_directory",
    "fs.sort_logical_paths",
    "ui.choose",
    "ui.notify",
    "ui.open_history",
    "ui.toggle_history",
    "ui.action_hint",
    "browser.profiles",
    "browser.call",
    "target.info",
    "target.context",
    "target.capture_foreground",
    "target.keyboard.shortcut",
    "target.keyboard.text",
    "keyboard.shortcut",
    "keyboard.sequence",
    "keyboard.text",
    "win32_edit.replace_selection",
    "raishot.resident.find",
    "raishot.resident.region",
    "raishot.capture_result",
    "raishot.release_result",
    "selection.copy",
    "selection.copy_data",
    "selection.read_text",
    "explorer.context",
    "clipboard.read",
    "clipboard.read_data",
    "clipboard.write",
    "clipboard.write_files",
    "window.action",
    "window.geometry",
    "window.set_rect",
    "pointer.state",
    "pointer.root",
    "target.handoff_pointing_root",
    "window.drag",
    "display.brightness_step",
    "audio.volume_adjust",
];

/// 由应用为本次调用捕获的本机上下文。不是沙盒或权限隔离。
/// 同步桥在动作工作线程执行；显式浏览器并发桥使用有界的作用域工作线程。
/// 回调须有界返回，并在副作用前核验取消和目标身份。
pub trait NativeHost: Send + Sync {
    /// 每次真实Service调用独立创建；未提供Hook所有权通道的宿主明确拒绝。
    fn create_owned_left_input(
        &self,
        _owner: crate::owned_mouse::OwnedInputOwner,
    ) -> Result<std::sync::Arc<crate::owned_mouse::OwnedLeftSession>, ActionError> {
        Err(crate::owned_mouse::fault(
            crate::owned_mouse::DOWN,
            "input_owner_unavailable",
            false,
        ))
    }
    /// 运行时完成、取消或异常后，在active标记释放和终态发布前收尾。
    fn finish_owned_inputs(&self) -> Result<(), ActionError> {
        Ok(())
    }

    /// 作用域包装本身不提供原有应用上下文；此标记不证明权限或设备可用。
    fn native_context_available(&self) -> bool {
        true
    }

    /// 只有服务为当前真实调用创建的作用域提供动作控制；默认宿主无此能力。
    fn action_scope_available(&self) -> bool {
        false
    }

    /// 明确选择浏览器并发转发；此标记不证明浏览器已连接或具备权限。
    fn browser_async_available(&self) -> bool {
        false
    }

    /// 只供显式浏览器并发桥使用；默认宿主不自动选择此路径。
    fn call_browser_async(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<Value, ActionError> {
        let reject = |code| {
            let mut error = ActionError::new(code, "此调用未提供异步浏览器转发。");
            error.method = Some(method.into());
            error.effect_state = Some("not_started".into());
            error
        };
        if !matches!(method, "browser.call" | "browser.profiles") || !params.is_object() {
            return Err(reject("invalid_input"));
        }
        if !self.browser_async_available() {
            return Err(reject("unsupported_runtime_feature"));
        }
        if cancellation.is_cancelled() {
            return Err(reject("cancelled"));
        }
        if Instant::now() >= deadline {
            return Err(reject("timeout"));
        }
        self.call_with_deadline(method, params, cancellation, deadline)
    }

    fn call(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError>;

    /// 长时间原生调用可据此遵守外层动作时限；默认沿用原有有界回调。
    fn call_with_deadline(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
        _deadline: Instant,
    ) -> Result<Value, ActionError> {
        self.call(method, params, cancellation)
    }
}

pub(crate) struct NativeBridge {
    listener: TcpListener,
    token: String,
    clients: Vec<Client>,
    next_id: u32,
}

struct Client {
    stream: TcpStream,
    began: Instant,
    input: Vec<u8>,
    output: Option<Vec<u8>>,
    written: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    token: String,
    id: u32,
    method: String,
    params: Value,
}

fn io_error(error: impl ToString) -> ActionError {
    let mut error = ActionError::new(
        "native_bridge_unavailable",
        format!("本机能力桥接不可用：{}", error.to_string()),
    );
    error.effect_state = Some("not_started".into());
    error
}

/// 交互 UI 和复制等待可能暂时停留在本次 poll 栈中，不能依赖父运行时
/// 继续轮询子进程才能发现宿主退出。旁路只检测连接和取消，不读走协议数据。
/// 无法强制抢占已经进入的阻塞系统调用；实现仍须在调用前后复验取消。
fn interactive_call(
    stream: &TcpStream,
    parent: &CancellationToken,
    call: impl FnOnce(&CancellationToken) -> Result<Value, ActionError>,
) -> Result<Value, ActionError> {
    let stream = stream.try_clone().map_err(io_error)?;
    let cancellation = CancellationToken::new();
    let done = AtomicBool::new(false);
    struct Finish<'a>(&'a AtomicBool);
    impl Drop for Finish<'_> {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    thread::scope(|scope| {
        let token = &cancellation;
        let stopped = &done;
        scope.spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                let mut byte = [0];
                let disconnected = match stream.peek(&mut byte) {
                    Ok(_) => true, // EOF，或违反一连接一帧约定的尾随数据。
                    Err(error) => error.kind() != std::io::ErrorKind::WouldBlock,
                };
                if parent.is_cancelled() || disconnected {
                    token.cancel();
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
        });
        let _finish = Finish(&done);
        if parent.is_cancelled() {
            cancellation.cancel();
        }
        call(&cancellation)
    })
}

impl NativeBridge {
    pub(crate) fn new() -> Result<Self, ActionError> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(io_error)?;
        listener.set_nonblocking(true).map_err(io_error)?;
        let mut random = [0u8; 32];
        getrandom::fill(&mut random).map_err(io_error)?;
        let token = random.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(Self {
            listener,
            token,
            clients: Vec::new(),
            next_id: 1,
        })
    }

    pub(crate) fn configuration(&self) -> Result<Value, ActionError> {
        Ok(
            json!({"port": self.listener.local_addr().map_err(io_error)?.port(), "token": self.token}),
        )
    }

    /// 每次只接纳一个连接，每客户端最多读取一帧；全部缓冲和在途连接有上限。
    #[cfg(test)]
    pub(crate) fn poll(&mut self, host: &dyn NativeHost, cancellation: &CancellationToken) {
        self.poll_until(host, cancellation, None);
    }

    pub(crate) fn poll_until(
        &mut self,
        host: &dyn NativeHost,
        cancellation: &CancellationToken,
        deadline: Option<Instant>,
    ) {
        if let Ok((stream, _)) = self.listener.accept() {
            if self.clients.len() < CLIENTS && stream.set_nonblocking(true).is_ok() {
                let _ = stream.set_nodelay(true);
                self.clients.push(Client {
                    stream,
                    began: Instant::now(),
                    input: Vec::new(),
                    output: None,
                    written: 0,
                });
            }
        }
        let token = &self.token;
        let next_id = &mut self.next_id;
        self.clients.retain_mut(|client| {
            let lifetime = if client.output.is_some() {
                Duration::from_secs(5)
            } else {
                Duration::from_secs(2)
            };
            if cancellation.is_cancelled() || client.began.elapsed() > lifetime {
                return false;
            }
            if client.output.is_none() {
                let mut bytes = [0u8; 4096];
                for _ in 0..16 {
                    match client.stream.read(&mut bytes) {
                        Ok(0) => return false,
                        Ok(count) => client.input.extend_from_slice(&bytes[..count]),
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(_) => return false,
                    }
                    if client.input.len() > LIMIT {
                        return false;
                    }
                    if client.input.contains(&b'\n') {
                        break;
                    }
                }
                if client.input.len() > LIMIT {
                    return false;
                }
                if let Some(end) = client.input.iter().position(|byte| *byte == b'\n') {
                    if end + 1 != client.input.len() {
                        return false;
                    }
                    let Ok(request) = serde_json::from_slice::<Request>(&client.input[..end])
                    else {
                        return false;
                    };
                    if request.token != *token || request.id != *next_id || request.id > 128 {
                        return false;
                    }
                    *next_id += 1;
                    let opening = matches!(
                        request.method.as_str(),
                        "system.open" | "system.open_file_in_parent"
                    );
                    let selection_stage_opt_in = request.method == "explorer.context"
                        && request.params.as_object().is_some_and(|params| {
                            params.len() == 1
                                && params.get("selection_failure_stage") == Some(&Value::Bool(true))
                        });
                    let pure_data = request.method == "fs.sort_logical_paths";
                    let file_read =
                        matches!(request.method.as_str(), "fs.stat" | "fs.read_directory");
                    let action_control = matches!(
                        request.method.as_str(),
                        "actions.stop_other_instances"
                            | "state.read"
                            | "state.compare_and_swap"
                            | "mouse.left_down"
                            | "mouse.left_up"
                    );
                    let capture = matches!(
                        request.method.as_str(),
                        "process.capture" | "raishot.capture_result" | "raishot.release_result"
                    );
                    let geometry = matches!(
                        request.method.as_str(),
                        "window.geometry"
                            | "window.set_rect"
                            | "pointer.state"
                            | "pointer.root"
                            | "window.drag"
                            | "target.handoff_pointing_root"
                    ) || request.method == "window.action"
                        && request.params.get("target").is_some();
                    let brightness = matches!(
                        request.method.as_str(),
                        "display.brightness_step" | "audio.volume_adjust"
                    );
                    let selection_read = matches!(
                        request.method.as_str(),
                        "selection.read_text" | "explorer.context"
                    );
                    let data_read = request.method == "clipboard.read_data" || selection_read;
                    let data_copy = request.method == "selection.copy_data";
                    let history_read =
                        matches!(request.method.as_str(), "history.list" | "history.get");
                    let history = matches!(
                        request.method.as_str(),
                        "history.list" | "history.get" | "history.append" | "history.remove"
                    );
                    let result = if !METHODS.contains(&request.method.as_str()) {
                        let mut error = ActionError::new("invalid_input", "未提供此本机调用方法。");
                        error.effect_state = Some("not_started".into());
                        Err(error)
                    } else if matches!(request.method.as_str(), "mouse.left_down" | "mouse.left_up")
                        && !host.action_scope_available()
                    {
                        Err(crate::owned_mouse::fault(
                            &request.method,
                            "actions_unavailable",
                            false,
                        ))
                    } else if request.method == "ui.action_hint" && !host.action_scope_available() {
                        Err(crate::action_hint::fault(
                            "ui_hint_scope_expired",
                            "not_started",
                        ))
                    } else if action_control && !host.action_scope_available() {
                        let mut error = ActionError::new(
                            "actions_unavailable",
                            "动作控制需要当前服务调用作用域。",
                        );
                        error.effect_state = Some("not_started".into());
                        error.method = Some(request.method.clone());
                        Err(error)
                    } else if pure_data && !cfg!(windows) {
                        let mut error =
                            ActionError::new("fs_unavailable", "本机字符串列表排序能力不可用。");
                        error.method = Some(request.method.clone());
                        error.effect_state = Some("not_started".into());
                        Err(error)
                    } else if geometry && !host.native_context_available() {
                        let mut error = ActionError::new(
                            "native_bridge_unavailable",
                            "窗口接口需要应用本机上下文。",
                        );
                        error.effect_state = Some("not_started".into());
                        error.method = Some(request.method.clone());
                        Err(error)
                    } else if !request.params.is_object()
                        || (request.method == "clipboard.read_data"
                            && request
                                .params
                                .as_object()
                                .is_some_and(|params| !params.is_empty()))
                    {
                        let mut error = ActionError::new("invalid_input", "本机调用参数无效。");
                        error.effect_state = Some("not_started".into());
                        Err(error)
                    } else if cancellation.is_cancelled() {
                        let mut error =
                            ActionError::new("cancelled", "动作已取消，未开始本机调用。");
                        if request.method == "win32_edit.replace_selection"
                            || request.method.starts_with("raishot.resident.")
                            || request.method == "clipboard.write_files"
                            || opening
                            || request.method == "ui.action_hint"
                            || file_read
                            || pure_data
                            || data_read
                            || data_copy
                            || history
                            || capture
                            || brightness
                            || geometry
                            || action_control
                        {
                            error.effect_state = Some("not_started".into());
                        }
                        Err(error)
                    } else {
                        let call = |cancellation: &CancellationToken| match deadline {
                            Some(deadline) => host.call_with_deadline(
                                &request.method,
                                request.params,
                                cancellation,
                                deadline,
                            ),
                            None => host.call(&request.method, request.params, cancellation),
                        };
                        if request.method == "win32_edit.replace_selection"
                            || request.method.starts_with("raishot.resident.")
                            || request.method == "clipboard.write_files"
                            || opening
                            || request.method == "ui.choose"
                            || matches!(
                                request.method.as_str(),
                                "target.capture_foreground"
                                    | "target.keyboard.shortcut"
                                    | "target.keyboard.text"
                            )
                            || request.method == "ui.open_history"
                            || request.method == "ui.toggle_history"
                            || request.method == "ui.action_hint"
                            || data_copy
                            || selection_read
                            || file_read
                            || history
                            || capture
                            || brightness
                            || geometry
                            || action_control
                        {
                            interactive_call(&client.stream, cancellation, call)
                        } else {
                            call(cancellation)
                        }
                    }
                    .map_err(|mut error| {
                        // 阶段码是可复制证明；必须检查原Host字段，不能借只读正规化补齐。
                        if request.method == "explorer.context"
                            && error.code == "selection_unavailable_after_directory"
                            && (!selection_stage_opt_in
                                || error.method.as_deref() != Some("explorer.context")
                                || error.effect_state.as_deref() != Some("not_started"))
                        {
                            let mut invalid = ActionError::new(
                                "invalid_response",
                                "资源管理器阶段错误字段不匹配或请求未明确启用。",
                            );
                            invalid.method = Some("explorer.context".into());
                            invalid.effect_state = Some("unknown".into());
                            return invalid;
                        }
                        let precise_class_fault =
                            preserve_class_rejection(&request.method, &mut error);
                        if geometry {
                            error.message =
                                "窗口矩形调用未获完整确认；请检查错误代码和效果状态。".into();
                            error.required_permissions.clear();
                            if !precise_class_fault {
                                error.method = Some(request.method.clone());
                                if matches!(
                                    request.method.as_str(),
                                    "window.geometry" | "pointer.state" | "pointer.root"
                                ) {
                                    error.effect_state = Some("not_started".into());
                                } else if error.effect_state.as_deref() != Some("not_started") {
                                    error.effect_state = Some("unknown".into());
                                }
                            }
                        }

                        if request.method == "win32_edit.replace_selection"
                            || request.method == "clipboard.write_files"
                            || request.method.starts_with("raishot.resident.")
                        {
                            error.message = if request.method.starts_with("raishot.resident.") {
                                "RaiShot 常驻请求未获完整确认；请检查错误代码和效果状态。".into()
                            } else if request.method == "clipboard.write_files" {
                                "文件列表剪贴板写入未获完整确认；请检查错误代码和效果状态。".into()
                            } else {
                                "Win32 Edit选区替换未获完整确认；请检查错误代码和效果状态。".into()
                            };
                            error.method = Some(request.method.clone());
                            error.required_permissions.clear();
                            if error.effect_state.as_deref() != Some("not_started") {
                                error.effect_state = Some("unknown".into());
                            }
                        }

                        if request.method == "ui.action_hint" {
                            error.message =
                                "动作版本提示更新未确认；请检查错误代码和效果状态。".into();
                            error.method = Some(request.method.clone());
                            error.required_permissions.clear();
                            if error.effect_state.as_deref() != Some("not_started") {
                                error.effect_state = Some("unknown".into());
                            }
                        }
                        if pure_data {
                            if !matches!(
                                error.code.as_str(),
                                "invalid_input" | "cancelled" | "timeout" | "fs_unavailable"
                            ) {
                                error.code = "fs_unavailable".into();
                            }
                            error.message = "字符串列表排序失败；请检查错误代码。".into();
                            error.method = Some(request.method.clone());
                            error.required_permissions.clear();
                            error.effect_state = Some("not_started".into());
                        }
                        if file_read {
                            error.message = "本地文件查询失败；请检查错误代码。".into();
                            error.method = Some(request.method.clone());
                            error.required_permissions.clear();
                            error.effect_state = Some("not_started".into());
                        }
                        if brightness {
                            error.message = if request.method == "audio.volume_adjust" {
                                "音量调节未获完整确认；请检查错误代码和效果状态。"
                            } else {
                                "亮度步进未获完整确认；请检查结构化回执。"
                            }
                            .into();
                            error.method = Some(request.method.clone());
                            error.required_permissions.clear();
                            if !matches!(
                                error.effect_state.as_deref(),
                                Some("not_started" | "unknown")
                            ) {
                                error.effect_state = Some("unknown".into());
                            }
                        }
                        if opening {
                            error.message = "打开请求未确认，请检查错误代码。".into();
                            error.method = Some(request.method.clone());
                            error.required_permissions.clear();
                            if error.effect_state.as_deref() != Some("not_started") {
                                error.effect_state = Some("unknown".into());
                            }
                        }
                        if capture {
                            error.message = "程序输出采集未完成，请检查错误代码。".into();
                            error.method = Some(request.method.clone());
                            error.required_permissions.clear();
                            if error.effect_state.as_deref() != Some("not_started") {
                                error.effect_state = Some("unknown".into());
                            }
                        } else if data_read || data_copy || history {
                            error.message = if history {
                                "剪贴板历史调用失败；请检查错误代码。"
                            } else {
                                "剪贴板或选区数据调用失败；请检查错误代码。"
                            }
                            .into();
                            error.method = Some(request.method.clone());
                            error.required_permissions.clear();
                            if data_read || history_read {
                                error.effect_state = Some("not_started".into());
                            } else if error.effect_state.as_deref() != Some("not_started") {
                                error.effect_state = Some("unknown".into());
                            }
                        }
                        error
                    });
                    let response = match result {
                        Ok(value) => json!({"id":request.id, "ok":true, "value":value}),
                        Err(error) => json!({"id":request.id, "ok":false, "error":error}),
                    };
                    let Ok(mut bytes) = serde_json::to_vec(&response) else {
                        return false;
                    };
                    let limit = if request.method.starts_with("browser.") {
                        BROWSER_RESPONSE_LIMIT
                    } else {
                        RESPONSE_LIMIT
                    };
                    if bytes.len() > limit {
                        return false;
                    }
                    bytes.push(b'\n');
                    client.output = Some(bytes);
                    client.began = Instant::now();
                }
            }
            if let Some(bytes) = &client.output {
                match client.stream.write(&bytes[client.written..]) {
                    Ok(0) => return false,
                    Ok(count) => client.written += count,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return true,
                    Err(_) => return false,
                }
                return client.written < bytes.len();
            }
            true
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn interactive_wait_observes_parent_cancel_and_host_disconnect_without_polling_runtime() {
        for disconnect in [false, true] {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let (stream, _) = listener.accept().unwrap();
            stream.set_nonblocking(true).unwrap();
            let parent = CancellationToken::new();
            let token = parent.clone();
            let (started, ready) = std::sync::mpsc::sync_channel(1);
            let worker = thread::spawn(move || {
                interactive_call(&stream, &token, |cancellation| {
                    started.send(()).unwrap();
                    let deadline = Instant::now() + Duration::from_secs(2);
                    while !cancellation.is_cancelled() && Instant::now() < deadline {
                        thread::sleep(Duration::from_millis(1));
                    }
                    assert!(
                        cancellation.is_cancelled(),
                        "交互窗口未观察到宿主断连或动作取消"
                    );
                    Ok(Value::Null)
                })
            });
            ready.recv().unwrap();
            if disconnect {
                drop(peer);
            } else {
                parent.cancel();
            }
            assert_eq!(worker.join().unwrap().unwrap(), Value::Null);
            // 断连仅取消本次交互调用，不污染可供动作服务查询的外层令牌。
            assert_eq!(parent.is_cancelled(), !disconnect);
        }
    }

    #[test]
    fn interactive_monitor_stops_after_a_normal_response() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let _peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        stream.set_nonblocking(true).unwrap();
        let value = interactive_call(&stream, &CancellationToken::new(), |_| {
            Ok(json!({"status":"selected","id":"stable"}))
        })
        .unwrap();
        assert_eq!(value["id"], "stable");
    }

    struct Host(AtomicUsize);
    impl NativeHost for Host {
        fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"submitted":true}))
        }
    }
    fn request(bridge: &NativeBridge, id: u32, token: &str) -> TcpStream {
        let mut client = TcpStream::connect(bridge.listener.local_addr().unwrap()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let frame =
            json!({"id":id,"token":token,"method":"keyboard.shortcut","params":{"keys":[65]}});
        writeln!(client, "{frame}").unwrap();
        client
    }

    fn drain(bridge: &mut NativeBridge, host: &Host, cancel: &CancellationToken) {
        let began = Instant::now();
        loop {
            bridge.poll(host, cancel);
            if bridge.clients.is_empty() {
                break;
            }
            assert!(began.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn native_bridge_authenticates_once_and_rejects_replay_oversize_and_cancelled_requests() {
        let mut bridge = NativeBridge::new().unwrap();
        let host = Host(AtomicUsize::new(0));
        let cancel = CancellationToken::new();
        let mut wrong = request(&bridge, 1, "wrong");
        drain(&mut bridge, &host, &cancel);
        assert_eq!(host.0.load(Ordering::SeqCst), 0);
        assert_eq!(wrong.read(&mut [0u8; 1]).unwrap(), 0);
        let mut accepted = request(&bridge, 1, &bridge.token);
        drain(&mut bridge, &host, &cancel);
        let mut response = String::new();
        accepted.read_to_string(&mut response).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&response).unwrap()["value"]["submitted"],
            true
        );
        assert_eq!(host.0.load(Ordering::SeqCst), 1);
        let _replay = request(&bridge, 1, &bridge.token);
        drain(&mut bridge, &host, &cancel);
        assert_eq!(host.0.load(Ordering::SeqCst), 1);
        let mut huge = TcpStream::connect(bridge.listener.local_addr().unwrap()).unwrap();
        let writer = std::thread::spawn(move || {
            let _ = huge.write_all(&vec![b'x'; LIMIT + 1]);
        });
        drain(&mut bridge, &host, &cancel);
        writer.join().unwrap();
        assert!(bridge.clients.is_empty());
        let _cancelled = request(&bridge, 2, &bridge.token);
        cancel.cancel();
        drain(&mut bridge, &host, &cancel);
        assert_eq!(host.0.load(Ordering::SeqCst), 1);
        let address = bridge.listener.local_addr().unwrap();
        drop(bridge);
        assert!(TcpStream::connect(address).is_err());
    }
    #[test]
    fn action_hint_cancelled_bridge_closes_pending_request_without_dispatch_or_wire_reply() {
        let mut bridge = NativeBridge::new().unwrap();
        let mut peer = TcpStream::connect(bridge.listener.local_addr().unwrap()).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        struct HintCount(AtomicUsize);
        impl NativeHost for HintCount {
            fn action_scope_available(&self) -> bool {
                true
            }
            fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(json!({}))
            }
        }
        let host = HintCount(AtomicUsize::new(0));
        let token = CancellationToken::new();
        // 先接纳没有帧的连接，避免把accept调度误当成取消回执的等待条件。
        let accept_deadline = Instant::now() + Duration::from_secs(2);
        loop {
            bridge.poll(&host, &token);
            if !bridge.clients.is_empty() {
                break;
            }
            assert!(Instant::now() < accept_deadline);
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(bridge.clients.len(), 1);
        assert_eq!(bridge.next_id, 1);
        let frame =
            json!({"id":1,"token":bridge.token,"method":"ui.action_hint","params":{"text":"n10"}});
        writeln!(peer, "{frame}").unwrap();
        token.cancel();
        bridge.poll(&host, &token);
        assert!(bridge.clients.is_empty());
        assert_eq!(bridge.next_id, 1, "取消先于解析，不能消耗请求ID");
        assert_eq!(host.0.load(Ordering::SeqCst), 0);
        // 未读请求随连接关闭；不同平台可返回EOF或reset，均不能期待结构化JSON。
        let mut response = Vec::new();
        match peer.read_to_end(&mut response) {
            Ok(count) => assert_eq!(count, 0),
            Err(error) => assert!(
                matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                ),
                "取消连接未关闭：{error}"
            ),
        }
        assert!(response.is_empty());
        assert!(!METHODS.contains(&"ui.action_hint.scoped"));
    }

    #[test]
    fn action_hint_actual_bridge_dispatch_observes_disconnect_and_parent_cancel() {
        struct HintHost {
            entered: std::sync::mpsc::SyncSender<()>,
            observed: AtomicBool,
        }
        impl NativeHost for HintHost {
            fn action_scope_available(&self) -> bool {
                true
            }
            fn call(
                &self,
                method: &str,
                _: Value,
                cancellation: &CancellationToken,
            ) -> Result<Value, ActionError> {
                assert_eq!(method, "ui.action_hint");
                self.entered.send(()).unwrap();
                let deadline = Instant::now() + Duration::from_secs(2);
                while !cancellation.is_cancelled() && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(1));
                }
                assert!(
                    cancellation.is_cancelled(),
                    "实际提示分派必须使用断连监视令牌"
                );
                self.observed.store(true, Ordering::Release);
                Err(crate::action_hint::fault("cancelled", "not_started"))
            }
        }
        for disconnect in [false, true] {
            let mut bridge = NativeBridge::new().unwrap();
            let mut peer = TcpStream::connect(bridge.listener.local_addr().unwrap()).unwrap();
            let frame = json!({"id":1,"token":bridge.token,"method":"ui.action_hint","params":{"text":"n10"}});
            writeln!(peer, "{frame}").unwrap();
            let (entered, ready) = std::sync::mpsc::sync_channel(1);
            let host = HintHost {
                entered,
                observed: AtomicBool::new(false),
            };
            let parent = CancellationToken::new();
            let token = parent.clone();
            let worker = thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(3);
                while !host.observed.load(Ordering::Acquire) {
                    bridge.poll(&host, &token);
                    assert!(Instant::now() < deadline);
                    thread::sleep(Duration::from_millis(1));
                }
            });
            ready.recv_timeout(Duration::from_secs(2)).unwrap();
            if disconnect {
                drop(peer);
            } else {
                parent.cancel();
            }
            worker.join().unwrap();
            assert_eq!(parent.is_cancelled(), !disconnect);
        }
    }
}

#[cfg(test)]
mod window_bridge_tests {
    use super::*;
    use std::io::BufRead;
    use std::sync::atomic::{AtomicUsize, Ordering};
    const WINDOW: [&str; 6] = [
        "window.geometry",
        "window.set_rect",
        "pointer.state",
        "pointer.root",
        "target.handoff_pointing_root",
        "window.drag",
    ];
    struct Host {
        available: bool,
        calls: AtomicUsize,
    }
    impl NativeHost for Host {
        fn native_context_available(&self) -> bool {
            self.available
        }
        fn action_scope_available(&self) -> bool {
            true
        }
        fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut error = ActionError::new("target_expired", "private-host-details");
            error.effect_state = Some("unknown".into());
            Err(error)
        }
    }
    fn exchange(
        host: &impl NativeHost,
        method: &str,
        params: Value,
        token: &CancellationToken,
    ) -> Value {
        let mut bridge = NativeBridge::new().unwrap();
        let mut peer = TcpStream::connect(bridge.listener.local_addr().unwrap()).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let frame = json!({"id":1,"token":bridge.token,"method":method,"params":params});
        writeln!(peer, "{frame}").unwrap();
        for _ in 0..16 {
            bridge.poll(host, token);
            thread::sleep(Duration::from_millis(1));
        }
        let mut line = String::new();
        std::io::BufReader::new(peer).read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }
    #[test]
    fn explorer_stage_bridge_requires_original_opt_in_and_exact_proof_fields() {
        struct StageHost {
            code: &'static str,
            method: Option<&'static str>,
            effect: Option<&'static str>,
            calls: AtomicUsize,
        }
        impl NativeHost for StageHost {
            fn call(
                &self,
                method: &str,
                _: Value,
                _: &CancellationToken,
            ) -> Result<Value, ActionError> {
                assert_eq!(method, "explorer.context");
                self.calls.fetch_add(1, Ordering::SeqCst);
                let mut error = ActionError::new(self.code, "private-input private-host-body");
                error.method = self.method.map(str::to_owned);
                error.effect_state = self.effect.map(str::to_owned);
                Err(error)
            }
        }
        let stage = "selection_unavailable_after_directory";
        for (name, params, code, method, effect, expected_code, expected_effect) in [
            (
                "qualified",
                json!({"selection_failure_stage":true}),
                stage,
                Some("explorer.context"),
                Some("not_started"),
                stage,
                "not_started",
            ),
            (
                "missing_method",
                json!({"selection_failure_stage":true}),
                stage,
                None,
                Some("not_started"),
                "invalid_response",
                "unknown",
            ),
            (
                "wrong_method",
                json!({"selection_failure_stage":true}),
                stage,
                Some("clipboard.read_data"),
                Some("not_started"),
                "invalid_response",
                "unknown",
            ),
            (
                "missing_effect",
                json!({"selection_failure_stage":true}),
                stage,
                Some("explorer.context"),
                None,
                "invalid_response",
                "unknown",
            ),
            (
                "unknown_effect",
                json!({"selection_failure_stage":true}),
                stage,
                Some("explorer.context"),
                Some("unknown"),
                "invalid_response",
                "unknown",
            ),
            (
                "default_request",
                json!({}),
                stage,
                Some("explorer.context"),
                Some("not_started"),
                "invalid_response",
                "unknown",
            ),
            (
                "explicit_false",
                json!({"selection_failure_stage":false}),
                stage,
                Some("explorer.context"),
                Some("not_started"),
                "invalid_response",
                "unknown",
            ),
            (
                "extra_options",
                json!({"selection_failure_stage":true,"extra":1}),
                stage,
                Some("explorer.context"),
                Some("not_started"),
                "invalid_response",
                "unknown",
            ),
            (
                "legacy_error",
                json!({}),
                "selection_unavailable",
                None,
                Some("unknown"),
                "selection_unavailable",
                "not_started",
            ),
        ] {
            let host = StageHost {
                code,
                method,
                effect,
                calls: AtomicUsize::new(0),
            };
            let response = exchange(&host, "explorer.context", params, &CancellationToken::new());
            assert_eq!(host.calls.load(Ordering::SeqCst), 1, "{name}");
            assert_eq!(response["ok"], false, "{name}: {response}");
            assert_eq!(
                response["error"]["code"], expected_code,
                "{name}: {response}"
            );
            assert_eq!(response["error"]["method"], "explorer.context", "{name}");
            assert_eq!(response["error"]["effect_state"], expected_effect, "{name}");
            assert!(!response.to_string().contains("private-input"), "{name}");
            assert!(
                !response.to_string().contains("private-host-body"),
                "{name}"
            );
        }
    }

    #[test]
    fn logical_sort_bridge_is_pure_data_without_native_target_and_redacts_failures() {
        struct PureHost(AtomicUsize);
        impl NativeHost for PureHost {
            fn native_context_available(&self) -> bool {
                false
            }
            fn call(
                &self,
                method: &str,
                params: Value,
                _: &CancellationToken,
            ) -> Result<Value, ActionError> {
                assert_eq!(method, "fs.sort_logical_paths");
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(params["paths"].clone())
            }
        }
        let host = PureHost(AtomicUsize::new(0));
        let paths = json!(["", "relative", "a\0tail", "relative"]);
        let response = exchange(
            &host,
            "fs.sort_logical_paths",
            json!({"paths":paths}),
            &CancellationToken::new(),
        );
        if cfg!(windows) {
            assert_eq!(response["ok"], true, "{response}");
            assert_eq!(response["value"], paths);
            assert_eq!(host.0.load(Ordering::SeqCst), 1);
        } else {
            assert_eq!(response["error"]["code"], "fs_unavailable");
            assert_eq!(host.0.load(Ordering::SeqCst), 0);
        }
        let failing = Host {
            available: false,
            calls: AtomicUsize::new(0),
        };
        let response = exchange(
            &failing,
            "fs.sort_logical_paths",
            json!({"paths":["private-input"]}),
            &CancellationToken::new(),
        );
        assert_eq!(response["error"]["code"], "fs_unavailable");
        assert_eq!(response["error"]["method"], "fs.sort_logical_paths");
        assert_eq!(response["error"]["effect_state"], "not_started");
        assert!(!response.to_string().contains("private-host-details"));
        assert!(!response.to_string().contains("private-input"));
    }

    #[test]
    fn window_methods_are_additive_and_reject_false_native_marker_before_dispatch() {
        let host = Host {
            available: false,
            calls: AtomicUsize::new(0),
        };
        assert_eq!(METHODS.len(), 52);
        for method in WINDOW {
            assert!(METHODS.contains(&method));
            let response = exchange(&host, method, json!({}), &CancellationToken::new());
            assert_eq!(response["error"]["code"], "native_bridge_unavailable");
            assert_eq!(response["error"]["effect_state"], "not_started");
        }
        let response = exchange(
            &host,
            "window.action",
            json!({"target":{},"operation":"toggle_maximize"}),
            &CancellationToken::new(),
        );
        assert_eq!(response["error"]["code"], "native_bridge_unavailable");
        assert_eq!(host.calls.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn window_read_and_write_faults_keep_structured_effect_and_hide_host_details() {
        let host = Host {
            available: true,
            calls: AtomicUsize::new(0),
        };
        for method in WINDOW {
            let response = exchange(&host, method, json!({}), &CancellationToken::new());
            let error = &response["error"];
            assert_eq!(error["code"], "target_expired");
            assert_eq!(error["method"], method);
            assert_eq!(
                error["effect_state"],
                if matches!(method, "window.geometry" | "pointer.state" | "pointer.root") {
                    "not_started"
                } else {
                    "unknown"
                }
            );
            assert!(!error["message"].as_str().unwrap().contains("private"));
            assert!(error
                .get("required_permissions")
                .is_none_or(|v| v.as_array().is_some_and(Vec::is_empty)));
        }
        assert_eq!(host.calls.load(Ordering::SeqCst), 6);
    }
    #[test]
    fn pre_cancelled_window_calls_do_not_enter_the_host() {
        let host = Host {
            available: true,
            calls: AtomicUsize::new(0),
        };
        for method in WINDOW {
            let mut bridge = NativeBridge::new().unwrap();
            let mut peer = TcpStream::connect(bridge.listener.local_addr().unwrap()).unwrap();
            peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
            let token = CancellationToken::new();
            let accept_deadline = Instant::now() + Duration::from_secs(1);
            while bridge.clients.is_empty() {
                bridge.poll(&host, &token);
                assert!(Instant::now() < accept_deadline);
            }
            writeln!(
                peer,
                "{}",
                json!({"id":1,"token":bridge.token,"method":method,"params":{}})
            )
            .unwrap();
            token.cancel();
            bridge.poll(&host, &token);
            assert!(bridge.clients.is_empty());
            let mut bytes = Vec::new();
            match peer.read_to_end(&mut bytes) {
                Ok(count) => assert_eq!(count, 0),
                Err(error) => assert!(matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                )),
            }
            assert!(bytes.is_empty());
        }
        assert_eq!(host.calls.load(Ordering::SeqCst), 0);
    }
}

#[cfg(test)]
mod class_rejection_bridge_tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpStream,
        thread,
        time::{Duration, Instant},
    };
    #[test]
    fn special_identity_and_original_effect_are_validated_before_read_normalization() {
        for method in [Some("window.geometry"), Some("pointer.root"), None] {
            for effect in [Some("not_started"), Some("unknown"), None] {
                let mut error = ActionError::new("window_class_blocked", "same");
                error.method = method.map(str::to_owned);
                error.effect_state = effect.map(str::to_owned);
                assert!(preserve_class_rejection("window.geometry", &mut error));
                let valid = method == Some("window.geometry") && effect.is_some();
                assert_eq!(
                    error.code,
                    if valid {
                        "window_class_blocked"
                    } else {
                        "invalid_response"
                    }
                );
                assert_eq!(
                    error.effect_state.as_deref(),
                    if valid { effect } else { Some("unknown") }
                );
            }
        }
        let mut other = ActionError::new("target_changed", "same");
        assert!(!preserve_class_rejection("window.geometry", &mut other));
        assert_eq!(other.code, "target_changed");
        assert!(other.effect_state.is_none());
    }
    struct FaultHost {
        method: Option<&'static str>,
        effect: Option<&'static str>,
    }
    impl NativeHost for FaultHost {
        fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
            let mut e = ActionError::new("window_class_blocked", "same");
            e.method = self.method.map(str::to_owned);
            e.effect_state = self.effect.map(str::to_owned);
            Err(e)
        }
    }
    #[test]
    fn actual_shared_socket_bridge_does_not_rewrite_the_special_evidence() {
        for method in [Some("window.geometry"), Some("pointer.root"), None] {
            for effect in [Some("not_started"), Some("unknown"), None] {
                let mut bridge = NativeBridge::new().unwrap();
                let config = bridge.configuration().unwrap();
                let mut peer =
                    TcpStream::connect(("127.0.0.1", config["port"].as_u64().unwrap() as u16))
                        .unwrap();
                let request =
                    json!({"id":1,"token":config["token"],"method":"window.geometry","params":{}});
                writeln!(peer, "{request}").unwrap();
                peer.set_nonblocking(true).unwrap();
                let host = FaultHost { method, effect };
                let cancellation = CancellationToken::new();
                let deadline = Instant::now() + Duration::from_secs(2);
                let mut output = Vec::new();
                loop {
                    bridge.poll(&host, &cancellation);
                    let mut bytes = [0u8; 4096];
                    match peer.read(&mut bytes) {
                        Ok(0) => {}
                        Ok(n) => output.extend_from_slice(&bytes[..n]),
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(e) => panic!("{e}"),
                    }
                    if output.ends_with(b"\n") {
                        break;
                    }
                    assert!(Instant::now() < deadline);
                    thread::sleep(Duration::from_millis(1));
                }
                let value: Value = serde_json::from_slice(&output).unwrap();
                let valid = method == Some("window.geometry") && effect.is_some();
                assert_eq!(
                    value["error"]["code"],
                    if valid {
                        "window_class_blocked"
                    } else {
                        "invalid_response"
                    }
                );
                assert_eq!(
                    value["error"]["effect_state"],
                    if valid { effect.unwrap() } else { "unknown" }
                );
                assert_eq!(value["error"]["method"], "window.geometry");
            }
        }
    }
}
