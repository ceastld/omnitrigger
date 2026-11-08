//! 两个引擎共用的宿主能力；这里不安装 Hook，不访问 GPUI。
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const MAX_TEXT: usize = 64 * 1024;

#[path = "process_capture.rs"]
mod process_capture;

fn capture_timeout() -> u64 {
    process_capture::Options::default().timeout_ms
}

fn capture_cwd<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
}

fn capture_api_error(failure: process_capture::Failure) -> ApiError {
    ApiError {
        code: failure.code.into(),
        message: failure.message.into(),
        effect_state: Some(failure.effect_state.into()),
        method: Some("process.capture".into()),
        required_permissions: Vec::new(),
    }
}

#[derive(Serialize, Debug)]
pub struct ApiError {
    pub(crate) code: String,
    pub(crate) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) effect_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) method: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) required_permissions: Vec<String>,
}

fn error(code: &'static str, message: impl ToString) -> ApiError {
    ApiError {
        code: code.into(),
        message: message.to_string(),
        effect_state: None,
        method: None,
        required_permissions: Vec::new(),
    }
}

fn open_parent_error(code: &str, started: bool) -> ApiError {
    ApiError {
        code: code.into(),
        message: "本地文件打开请求未确认。".into(),
        effect_state: Some(if started { "unknown" } else { "not_started" }.into()),
        method: Some("system.open_file_in_parent".into()),
        required_permissions: Vec::new(),
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    Delay {
        milliseconds: u64,
    },
    Native {
        connection: crate::native::Configuration,
        id: u32,
        method: String,
        params: Value,
    },
    ReadText {
        path: String,
    },
    WriteText {
        path: String,
        text: String,
        overwrite: bool,
    },
    Run {
        program: String,
        args: Vec<String>,
        timeout_ms: u64,
    },
    Capture {
        program: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default = "capture_timeout")]
        timeout_ms: u64,
        #[serde(default)]
        allow_non_zero: bool,
        #[serde(default, deserialize_with = "capture_cwd")]
        cwd: Option<String>,
    },
    Start {
        program: String,
        args: Vec<String>,
    },
    ReadClipboard,
    WriteClipboard {
        text: String,
    },
    Open {
        target: String,
    },
    OpenFileInParent {
        target: String,
    },
}

/// 调用来源策略；它不是脚本安全沙箱，文件/进程能力仍属于可信脚本。
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DesktopMode {
    Standalone,
    ApplicationBridge,
}

impl DesktopMode {
    pub(crate) fn resolve(
        explicit: Option<Self>,
        context: Option<&Value>,
    ) -> Result<Self, ApiError> {
        let legacy_binding = context.is_some_and(|context| {
            context.get("connection").is_some()
                || context.get("browser_async_connection").is_some()
                || context
                    .get("error")
                    .and_then(|error| error.get("code"))
                    .and_then(Value::as_str)
                    .is_some_and(|code| code != "desktop_unavailable")
        });
        match explicit {
            Some(Self::Standalone) if legacy_binding => Err(ApiError {
                code: "invalid_request".into(),
                message: "Standalone策略不能覆盖已声明的应用桥连接或失败。".into(),
                effect_state: Some("not_started".into()),
                method: None,
                required_permissions: Vec::new(),
            }),
            Some(mode) => Ok(mode),
            None => Ok(if legacy_binding {
                Self::ApplicationBridge
            } else {
                Self::Standalone
            }),
        }
    }
    pub(crate) fn local_allowed(self) -> bool {
        self == Self::Standalone
    }
}

thread_local! {
    // 独立CLI沿用旧桌面能力；每次应用请求在进入解释器前显式关闭。
    static LOCAL_DESKTOP_ALLOWED: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}
pub(crate) struct DesktopScope {
    previous: bool,
}
impl DesktopScope {
    pub(crate) fn enter(mode: DesktopMode) -> Self {
        let previous = LOCAL_DESKTOP_ALLOWED.with(|value| value.replace(mode.local_allowed()));
        Self { previous }
    }
}
impl Drop for DesktopScope {
    fn drop(&mut self) {
        LOCAL_DESKTOP_ALLOWED.with(|value| value.set(self.previous));
    }
}

fn local_desktop_method(call: &Call) -> Option<&'static str> {
    match call {
        Call::Capture { .. } => Some("process.capture"),
        Call::Open { .. } => Some("system.open"),
        Call::OpenFileInParent { .. } => Some("system.open_file_in_parent"),
        Call::ReadClipboard => Some("clipboard.read"),
        Call::WriteClipboard { .. } => Some("clipboard.write"),
        _ => None,
    }
}
fn check_local_desktop(call: &Call) -> Result<(), ApiError> {
    if let Some(method) = local_desktop_method(call) {
        if !LOCAL_DESKTOP_ALLOWED.with(std::cell::Cell::get) {
            return Err(ApiError {
                code: "local_desktop_forbidden".into(),
                message: "应用桥调用禁止退回宿主本地桌面API。".into(),
                effect_state: Some("not_started".into()),
                method: Some(method.into()),
                required_permissions: Vec::new(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod desktop_policy_tests {
    use super::*;
    #[test]
    fn application_capture_cannot_use_local_kind_but_standalone_policy_is_preserved() {
        let call = serde_json::from_value::<Call>(
            json!({"kind":"capture","program":"C:/Owned/never.exe"}),
        )
        .unwrap();
        {
            let _scope = DesktopScope::enter(DesktopMode::ApplicationBridge);
            let error = check_local_desktop(&call).unwrap_err();
            assert_eq!(error.code, "local_desktop_forbidden");
            assert_eq!(error.method.as_deref(), Some("process.capture"));
            assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        }
        assert!(check_local_desktop(&call).is_ok());
        // 只检查纯策略，绝不调用dispatch/execute或启动never.exe。
    }

    #[test]
    fn desktop_policy_uses_binding_evidence_and_explicit_producer_mode() {
        for context in [
            Value::Null,
            json!({"error":{"code":"desktop_unavailable"}}),
            json!({"unrelated":true}),
        ] {
            assert_eq!(
                DesktopMode::resolve(None, Some(&context)).unwrap(),
                DesktopMode::Standalone
            );
            assert_eq!(
                DesktopMode::resolve(Some(DesktopMode::ApplicationBridge), Some(&context)).unwrap(),
                DesktopMode::ApplicationBridge
            );
        }
        assert_eq!(
            DesktopMode::resolve(None, None).unwrap(),
            DesktopMode::Standalone
        );
        for context in [
            json!({"connection":{},"native_context_available":false}),
            json!({"error":{"code":"native_bridge_unavailable"}}),
        ] {
            assert_eq!(
                DesktopMode::resolve(None, Some(&context)).unwrap(),
                DesktopMode::ApplicationBridge
            );
            assert!(DesktopMode::resolve(Some(DesktopMode::Standalone), Some(&context)).is_err());
        }
    }

    #[test]
    fn desktop_policy_stops_all_four_legacy_calls_without_executing_them() {
        let _scope = DesktopScope::enter(DesktopMode::ApplicationBridge);
        for (call, method) in [
            (
                Call::Open {
                    target: "unused".into(),
                },
                "system.open",
            ),
            (
                Call::OpenFileInParent {
                    target: "unused".into(),
                },
                "system.open_file_in_parent",
            ),
            (Call::ReadClipboard, "clipboard.read"),
            (
                Call::WriteClipboard {
                    text: "unused".into(),
                },
                "clipboard.write",
            ),
        ] {
            // 只调用纯门函数，不测试真实desktop::函数或dispatch的副作用路径。
            let error = check_local_desktop(&call).unwrap_err();
            assert_eq!(error.code, "local_desktop_forbidden");
            assert_eq!(error.effect_state.as_deref(), Some("not_started"));
            assert_eq!(error.method.as_deref(), Some(method));
        }
        assert!(check_local_desktop(&Call::ReadText {
            path: "unused".into()
        })
        .is_ok());
        assert!(check_local_desktop(&Call::Run {
            program: "unused".into(),
            args: vec![],
            timeout_ms: 1
        })
        .is_ok());
    }

    #[test]
    fn desktop_policy_restores_scope_and_does_not_promote_false_availability() {
        assert!(LOCAL_DESKTOP_ALLOWED.with(std::cell::Cell::get));
        let context = json!({"connection":{},"native_context_available":false});
        {
            let mode = DesktopMode::resolve(None, Some(&context)).unwrap();
            let _scope = DesktopScope::enter(mode);
            assert!(!LOCAL_DESKTOP_ALLOWED.with(std::cell::Cell::get));
            assert_eq!(context["native_context_available"], false);
        }
        assert!(LOCAL_DESKTOP_ALLOWED.with(std::cell::Cell::get));
    }
}

pub fn dispatch(request: String) -> String {
    let began = Instant::now();
    let result = dispatch_inner(request);
    exclude_host_wait(began.elapsed());
    result
}

fn dispatch_inner(request: String) -> String {
    let open_parent_request = request.len() <= 512 * 1024
        && serde_json::from_str::<Value>(&request)
            .is_ok_and(|value| value["kind"] == "open_file_in_parent");
    let capture_request = request.len() <= 512 * 1024
        && request.contains("\"capture\"")
        && serde_json::from_str::<Value>(&request).is_ok_and(|value| value["kind"] == "capture");
    let result = if request.len() > 512 * 1024 {
        Err(error("request_too_large", "API 请求超过 512 KiB。"))
    } else {
        serde_json::from_str::<Call>(&request)
            .map_err(|cause| {
                if capture_request {
                    capture_api_error(process_capture::fault("invalid_arguments", false))
                } else if open_parent_request {
                    open_parent_error("invalid_arguments", false)
                } else {
                    error("invalid_arguments", cause)
                }
            })
            .and_then(|call| {
                if request.len() > 256 * 1024 && !matches!(call, Call::Native { .. }) {
                    return Err(error("request_too_large", "API 请求超过 256 KiB。"));
                }
                check_local_desktop(&call)?;
                execute(call)
            })
    };
    match result {
        Ok(value) => json!({"ok":true,"value":value}).to_string(),
        Err(cause) => json!({"ok":false,"error":cause}).to_string(),
    }
}

fn absolute_path(path: &str) -> Result<&Path, ApiError> {
    let path = Path::new(path);
    if !path.is_absolute() {
        return Err(error("invalid_path", "文件路径必须为绝对路径。"));
    }
    Ok(path)
}

fn read_text(path: &str) -> Result<String, ApiError> {
    let file =
        File::open(absolute_path(path)?).map_err(|cause| error("file_read_failed", cause))?;
    let mut bytes = Vec::new();
    file.take(MAX_TEXT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|cause| error("file_read_failed", cause))?;
    if bytes.len() > MAX_TEXT {
        return Err(error("text_too_large", "文件超过 64 KiB。"));
    }
    String::from_utf8(bytes).map_err(|cause| error("invalid_utf8", cause))
}

fn check_text(text: &str) -> Result<(), ApiError> {
    if text.len() > MAX_TEXT {
        return Err(error("text_too_large", "文本超过 64 KiB。"));
    }
    Ok(())
}

fn command(program: &str, args: &[String]) -> Result<Command, ApiError> {
    if program.trim().is_empty()
        || program.contains('\0')
        || args.iter().any(|arg| arg.contains('\0'))
    {
        return Err(error(
            "invalid_arguments",
            "程序与参数不能包含 NUL，程序不能为空。",
        ));
    }
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    Ok(command)
}

struct ManagedChild(Child);
impl Drop for ManagedChild {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn execute(call: Call) -> Result<Value, ApiError> {
    match call {
        Call::Delay { milliseconds } => {
            if milliseconds > 60_000 {
                return Err(error(
                    "invalid_arguments",
                    "等待时间必须为 0–60000 毫秒的整数。",
                ));
            }
            thread::sleep(Duration::from_millis(milliseconds));
            Ok(Value::Null)
        }
        Call::Native {
            connection,
            id,
            method,
            params,
        } => crate::native::call(connection, id, method, params),
        Call::ReadText { path } => Ok(json!(read_text(&path)?)),
        Call::WriteText {
            path,
            text,
            overwrite,
        } => {
            check_text(&text)?;
            let mut options = OpenOptions::new();
            options.write(true);
            if overwrite {
                options.create(true).truncate(true);
            } else {
                options.create_new(true);
            }
            let mut file = options
                .open(absolute_path(&path)?)
                .map_err(|cause| error("file_write_failed", cause))?;
            file.write_all(text.as_bytes())
                .map_err(|cause| error("file_write_failed", cause))?;
            Ok(Value::Null)
        }
        Call::Run {
            program,
            args,
            timeout_ms,
        } => {
            if !(1..=1000).contains(&timeout_ms) {
                return Err(error(
                    "invalid_arguments",
                    "原型进程等待上限为 1–1000 毫秒。",
                ));
            }
            let mut child = ManagedChild(
                command(&program, &args)?
                    .spawn()
                    .map_err(|cause| error("process_start_failed", cause))?,
            );
            let pid = child.0.id();
            let deadline = Instant::now() + Duration::from_millis(timeout_ms);
            loop {
                if let Some(status) = child
                    .0
                    .try_wait()
                    .map_err(|cause| error("process_wait_failed", cause))?
                {
                    return Ok(
                        json!({"pid":pid,"exitCode":status.code(),"success":status.success()}),
                    );
                }
                if Instant::now() >= deadline {
                    return Err(error(
                        "process_timeout",
                        "进程等待超时，已请求终止直接子进程。",
                    ));
                }
                thread::sleep(Duration::from_millis(5));
            }
        }
        Call::Capture {
            program,
            args,
            timeout_ms,
            allow_non_zero,
            cwd,
        } => {
            let result = process_capture::capture(
                &program,
                &args,
                &process_capture::Options {
                    timeout_ms,
                    allow_non_zero,
                    cwd,
                },
            );
            result.map(|value| json!(value)).map_err(capture_api_error)
        }
        Call::Start { program, args } => {
            let child = command(&program, &args)?
                .spawn()
                .map_err(|cause| error("process_start_failed", cause))?;
            let pid = child.id();
            // 后台回收，不等待独立程序退出；独立程序不随脚本结束而关闭。
            thread::spawn(move || {
                let mut child = child;
                let _ = child.wait();
            });
            Ok(json!({"pid":pid}))
        }
        Call::ReadClipboard => desktop::read_clipboard().map(Value::String),
        Call::WriteClipboard { text } => {
            check_text(&text)?;
            desktop::write_clipboard(&text)?;
            Ok(Value::Null)
        }
        Call::Open { target } => {
            desktop::open(&target)?;
            Ok(Value::Null)
        }
        Call::OpenFileInParent { target } => {
            desktop::open_file_in_parent(&target)?;
            Ok(Value::Null)
        }
    }
}

#[cfg(windows)]
mod desktop {
    use super::*;
    pub fn read_clipboard() -> Result<String, ApiError> {
        let text = omnitrigger_windows::read_clipboard_text()
            .map_err(|cause| error("clipboard_read_failed", cause))?;
        check_text(&text)?;
        Ok(text)
    }
    pub fn write_clipboard(text: &str) -> Result<(), ApiError> {
        omnitrigger_windows::write_clipboard_text(text)
            .map_err(|cause| error("clipboard_write_failed", cause))
    }
    pub fn open(target: &str) -> Result<(), ApiError> {
        omnitrigger_windows::open_target(target).map_err(|cause| error("open_failed", cause))
    }
    pub fn open_file_in_parent(target: &str) -> Result<(), ApiError> {
        omnitrigger_windows::open_file_in_parent(target)
            .map_err(|failure| open_parent_error(failure.code, failure.started))
    }
}

#[cfg(not(windows))]
mod desktop {
    use super::*;
    pub fn read_clipboard() -> Result<String, ApiError> {
        Err(error("unsupported_platform", "此 API 需要 Windows。"))
    }
    pub fn write_clipboard(_: &str) -> Result<(), ApiError> {
        read_clipboard().map(|_| ())
    }
    pub fn open(_: &str) -> Result<(), ApiError> {
        read_clipboard().map(|_| ())
    }
    pub fn open_file_in_parent(_: &str) -> Result<(), ApiError> {
        Err(open_parent_error("unsupported_platform", false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_parent_wire_rejects_shape_before_any_desktop_call() {
        for request in [
            json!({"kind":"open_file_in_parent"}),
            json!({"kind":"open_file_in_parent","target":null}),
            json!({"kind":"open_file_in_parent","target":123}),
            json!({"kind":"open_file_in_parent","target":[]}),
            json!({"kind":"open_file_in_parent","target":"C:\\Synthetic\\a.pdf","directory":"secret-extra"}),
            json!({"kind":"open_file_in_parent","target":"C:\\Synthetic\\a.pdf","args":[]}),
        ] {
            let response: Value = serde_json::from_str(&dispatch(request.to_string())).unwrap();
            assert_eq!(response["ok"], false);
            assert_eq!(response["error"]["code"], "invalid_arguments");
            assert_eq!(response["error"]["effect_state"], "not_started");
            assert_eq!(response["error"]["method"], "system.open_file_in_parent");
            assert!(!response.to_string().contains("secret-extra"));
        }
    }

    #[test]
    fn open_parent_wire_deserialization_preserves_percent_and_non_bmp_without_execution() {
        let expected = "C:\\合成😀\\percent%20%NAME%.pdf";
        let request = json!({"kind":"open_file_in_parent","target":expected});
        let Call::OpenFileInParent { target } = serde_json::from_value::<Call>(request).unwrap()
        else {
            panic!("应解析成独立文件父目录打开请求");
        };
        assert_eq!(target, expected);
        let error = open_parent_error("open_failed", true);
        assert_eq!(error.effect_state.as_deref(), Some("unknown"));
        assert_eq!(error.method.as_deref(), Some("system.open_file_in_parent"));
    }

    #[test]
    fn delay_wire_rejects_invalid_values_without_relying_on_sdk_validation() {
        for value in [
            json!(60001),
            json!(-1),
            json!(1.5),
            json!("1"),
            json!(null),
            json!(true),
        ] {
            let request = json!({"kind":"delay","milliseconds":value});
            let response: Value = serde_json::from_str(&dispatch(request.to_string())).unwrap();
            assert_eq!(response["ok"], false);
            assert_eq!(response["error"]["code"], "invalid_arguments");
        }
        let response: Value = serde_json::from_str(&dispatch(
            json!({"kind":"delay","milliseconds":0}).to_string(),
        ))
        .unwrap();
        assert_eq!(response, json!({"ok":true,"value":null}));
    }

    #[test]
    fn capture_wire_rejects_invalid_arguments_before_any_program_start() {
        for extra in [
            json!({"timeout_ms":0}),
            json!({"timeout_ms":60001}),
            json!({"timeout_ms":"1"}),
            json!({"allow_non_zero":1}),
            json!({"unknown":true}),
            json!({"cwd":null}),
            json!({"args":"secret-output"}),
        ] {
            let mut request = json!({"kind":"capture","program":"C:\\fixture.exe","args":[]});
            for (key, value) in extra.as_object().unwrap() {
                request[key] = value.clone();
            }
            let response: Value = serde_json::from_str(&dispatch(request.to_string())).unwrap();
            assert_eq!(response["ok"], false);
            assert_eq!(response["error"]["code"], "invalid_arguments");
            assert_eq!(response["error"]["effect_state"], "not_started");
            assert!(!response.to_string().contains("secret-output"));
        }
    }
}

thread_local! {
    static BROWSER_ASYNC_POOL: std::cell::RefCell<Option<crate::native::BrowserAsyncPool>> = const { std::cell::RefCell::new(None) };
    static HOST_WAIT_MICROS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

pub(crate) fn excluded_host_micros() -> u64 {
    HOST_WAIT_MICROS.with(|time| time.get())
}
pub(crate) fn exclude_host_wait(elapsed: Duration) {
    let micros = elapsed.as_micros().min(u128::from(u64::MAX)) as u64;
    HOST_WAIT_MICROS.with(|time| time.set(time.get().saturating_add(micros)));
}

pub(crate) struct AsyncScope;
impl AsyncScope {
    pub(crate) fn enter(context: Option<&Value>) -> Self {
        BROWSER_ASYNC_POOL.with(|pool| {
            // 仅保存配置值。旧父进程缺字段时仍可跑同步入口，但异步请求在IO前拒绝。
            *pool.borrow_mut() = Some(crate::native::BrowserAsyncPool::new(
                context
                    .and_then(|value| value.get("browser_async_connection"))
                    .cloned(),
            ));
        });
        Self
    }
}
impl Drop for AsyncScope {
    fn drop(&mut self) {
        let pool = BROWSER_ASYNC_POOL.with(|pool| pool.borrow_mut().take());
        drop(pool); // 标记取消并join本次所有工作线程；没有遗留到下次请求的池。
    }
}
pub(crate) fn dispatch_async(request: String) -> String {
    BROWSER_ASYNC_POOL.with(|pool| {
        if let Some(pool) = &mut *pool.borrow_mut() { pool.dispatch(request) }
        else { json!({"ok":false,"error":{"code":"browser_async_unavailable","message":"没有本次异步浏览器作用域。","effect_state":"not_started","method":"browser.call"}}).to_string() }
    })
}
pub(crate) fn poll_async() -> String {
    BROWSER_ASYNC_POOL.with(|pool| {
        pool.borrow_mut()
            .as_mut()
            .map_or_else(|| "null".into(), |pool| pool.poll())
    })
}
pub(crate) fn async_pending() -> usize {
    BROWSER_ASYNC_POOL.with(|pool| pool.borrow().as_ref().map_or(0, |pool| pool.pending()))
}
pub(crate) fn wait_async() {
    let began = Instant::now();
    BROWSER_ASYNC_POOL.with(|pool| {
        if let Some(pool) = &mut *pool.borrow_mut() {
            pool.wait(Duration::from_millis(25));
        }
    });
    exclude_host_wait(began.elapsed());
}
