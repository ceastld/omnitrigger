//! 应用向脚本提供的按次桌面上下文；不在 Hook 中读取进程或发送输入。
use omnitrigger_actions::{ActionError, CancellationToken, NativeHost};
use omnitrigger_windows::{capture_shortcut_target, ScriptTarget, ShortcutTarget};
use serde::Deserialize;
use serde_json::{json, Value};

pub struct DesktopHost {
    target: Result<ScriptTarget, ActionError>,
}

fn error(code: &str, message: impl ToString, effect: &str) -> ActionError {
    let mut error = ActionError::new(code, message.to_string());
    error.effect_state = Some(effect.into());
    error
}

impl DesktopHost {
    pub fn capture(target: Option<ShortcutTarget>) -> Self {
        // 捕获失败只影响桌面调用，浏览器或纯代码动作仍可运行。
        let target = target
            .map(Ok)
            .unwrap_or_else(capture_shortcut_target)
            .and_then(ScriptTarget::capture)
            .map_err(|message| error("target_unavailable", message, "not_started"));
        Self { target }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShortcutArgs {
    keys: Vec<u16>,
    #[serde(default)]
    key_down_delay_ms: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SequenceArgs {
    modifiers: Vec<u16>,
    keys: Vec<u16>,
    #[serde(default)]
    key_down_delay_ms: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextArgs {
    text: String,
}

fn not_cancelled(cancellation: &CancellationToken) -> Result<(), String> {
    if cancellation.is_cancelled() {
        Err("动作已取消，未继续本机调用。".into())
    } else {
        Ok(())
    }
}

impl NativeHost for DesktopHost {
    fn call(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError> {
        if cancellation.is_cancelled() {
            return Err(error(
                "cancelled",
                "动作已取消，未开始本机调用。",
                "not_started",
            ));
        }
        match method {
            "clipboard.read" => {
                if params != json!({}) {
                    return Err(error(
                        "invalid_input",
                        "剪贴板读取不接受参数。",
                        "not_started",
                    ));
                }
                let text = omnitrigger_windows::read_clipboard_text()
                    .map_err(|message| error("clipboard_failed", message, "not_started"))?;
                not_cancelled(cancellation)
                    .map_err(|message| error("cancelled", message, "not_started"))?;
                Ok(json!(text))
            }
            "clipboard.write" | "keyboard.text" => {
                let args: TextArgs = serde_json::from_value(params)
                    .map_err(|message| error("invalid_input", message, "not_started"))?;
                omnitrigger_windows::validate_automation_text(&args.text)
                    .map_err(|message| error("invalid_input", message, "not_started"))?;
                if method == "clipboard.write" {
                    omnitrigger_windows::write_clipboard_text_if(&args.text, || {
                        not_cancelled(cancellation)
                    })
                    .map_err(|message| error("clipboard_failed", message, "unknown"))?;
                    Ok(json!({"written":true}))
                } else {
                    let target = self.target.as_ref().map_err(Clone::clone)?;
                    target
                        .ensure_foreground()
                        .map_err(|message| error("target_changed", message, "not_started"))?;
                    target
                        .send_text_if(&args.text, || not_cancelled(cancellation))
                        .map_err(|message| error("input_failed", message, "unknown"))?;
                    Ok(json!({"submitted":true}))
                }
            }
            "selection.copy" => {
                if params != json!({}) {
                    return Err(error(
                        "invalid_input",
                        "复制选区不接受参数。",
                        "not_started",
                    ));
                }
                let target = self.target.as_ref().map_err(Clone::clone)?;
                target
                    .ensure_foreground()
                    .map_err(|message| error("target_changed", message, "not_started"))?;
                let text = target
                    .copy_selection_text_if(|| not_cancelled(cancellation))
                    .map_err(|message| error("selection_failed", message, "unknown"))?;
                Ok(json!(text))
            }
            "target.info" => {
                if params != json!({}) {
                    return Err(error(
                        "invalid_input",
                        "目标信息查询不接受参数。",
                        "not_started",
                    ));
                }
                let info = self
                    .target
                    .as_ref()
                    .map_err(Clone::clone)?
                    .info()
                    .map_err(|message| error("target_changed", message, "not_started"))?;
                Ok(
                    json!({"processName":info.process_name,"windowRole":info.window_role.as_str(),"title":info.title,"processId":info.process_id,"isForeground":info.is_foreground}),
                )
            }
            "keyboard.shortcut" => {
                let args: ShortcutArgs = serde_json::from_value(params)
                    .map_err(|message| error("invalid_input", message, "not_started"))?;
                omnitrigger_windows::validate_shortcut(&args.keys)
                    .map_err(|message| error("invalid_input", message, "not_started"))?;
                omnitrigger_windows::validate_key_timing(1, args.key_down_delay_ms)
                    .map_err(|message| error("invalid_input", message, "not_started"))?;
                let target = self.target.as_ref().map_err(Clone::clone)?;
                target
                    .ensure_foreground()
                    .map_err(|message| error("target_changed", message, "not_started"))?;
                if cancellation.is_cancelled() {
                    return Err(error("cancelled", "动作已取消。", "not_started"));
                }
                target
                    .send_shortcut_timed_if(&args.keys, args.key_down_delay_ms, || {
                        if cancellation.is_cancelled() {
                            Err("动作已取消，未提交输入。".into())
                        } else {
                            Ok(())
                        }
                    })
                    .map_err(|message| error("input_failed", message, "unknown"))?;
                Ok(json!({"submitted":true}))
            }
            "keyboard.sequence" => {
                let args: SequenceArgs = serde_json::from_value(params)
                    .map_err(|message| error("invalid_input", message, "not_started"))?;
                omnitrigger_windows::validate_key_sequence(&args.modifiers, &args.keys)
                    .and_then(|_| {
                        omnitrigger_windows::validate_key_timing(
                            args.keys.len(),
                            args.key_down_delay_ms,
                        )
                    })
                    .map_err(|message| error("invalid_input", message, "not_started"))?;
                let target = self.target.as_ref().map_err(Clone::clone)?;
                target
                    .ensure_foreground()
                    .map_err(|message| error("target_changed", message, "not_started"))?;
                target
                    .send_key_sequence_if(
                        &args.modifiers,
                        &args.keys,
                        args.key_down_delay_ms,
                        || not_cancelled(cancellation),
                    )
                    .map_err(|message| error("input_failed", message, "unknown"))?;
                Ok(json!({"submitted":true}))
            }
            _ => Err(error(
                "unknown_method",
                "本机宿主不支持此方法。",
                "not_started",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_validation_and_cancellation_do_not_fall_back_to_another_target() {
        let host = DesktopHost {
            target: Err(error(
                "target_unavailable",
                "测试目标不存在。",
                "not_started",
            )),
        };
        let token = CancellationToken::new();
        for (method, params) in [
            (
                "keyboard.shortcut",
                json!({"keys":[65],"key_down_delay_ms":1001}),
            ),
            ("keyboard.sequence", json!({"modifiers":[17],"keys":[65]})),
            ("keyboard.sequence", json!({"modifiers":[162],"keys":[]})),
            (
                "keyboard.sequence",
                json!({"modifiers":[162],"keys":vec![65;16],"key_down_delay_ms":200}),
            ),
            (
                "keyboard.sequence",
                json!({"modifiers":[],"keys":[65],"extra":1}),
            ),
            ("clipboard.read", json!({"unexpected":true})),
            ("clipboard.write", json!({"text":"a\0b"})),
            ("keyboard.text", json!({"text":5})),
            ("selection.copy", json!([])),
        ] {
            assert_eq!(
                host.call(method, params, &token).unwrap_err().code,
                "invalid_input"
            );
        }
        assert_eq!(
            host.call("keyboard.shortcut", json!({"keys":[65,162]}), &token)
                .unwrap_err()
                .code,
            "invalid_input"
        );
        assert_eq!(
            host.call("target.info", json!({}), &token)
                .unwrap_err()
                .code,
            "target_unavailable"
        );
        assert_eq!(
            host.call("target.info", json!({"other":true}), &token)
                .unwrap_err()
                .code,
            "invalid_input"
        );
        assert_eq!(
            host.call("other", json!({}), &token).unwrap_err().code,
            "unknown_method"
        );
        token.cancel();
        assert_eq!(
            host.call("keyboard.shortcut", json!({"keys":[65]}), &token)
                .unwrap_err()
                .code,
            "cancelled"
        );
    }

    /// 显式在本机交互桌面运行；只向本测试创建的输入框注入按键。
    #[test]
    #[ignore = "短暂激活专用输入框，验证 Node 桌面 API 的真实效果"]
    fn interactive_desktop_api_smoke() {
        use omnitrigger_actions::{
            ActionCall, ActionOutcome, ActionService, NodeRuntime, PackageCatalog, RuntimeRegistry,
        };
        use std::{
            sync::Arc,
            thread,
            time::{Duration, Instant},
        };
        use windows::{
            core::w,
            Win32::{
                Foundation::{HWND, LPARAM, WPARAM},
                UI::{
                    Input::KeyboardAndMouse::SetFocus,
                    WindowsAndMessaging::{
                        CreateWindowExW, DestroyWindow, DispatchMessageW, GetForegroundWindow,
                        GetWindowTextW, PeekMessageW, SendMessageW, SetForegroundWindow,
                        SetWindowTextW, ShowWindow, TranslateMessage, MSG, PM_REMOVE,
                        SW_SHOWNOACTIVATE, WINDOW_EX_STYLE, WINDOW_STYLE, WS_BORDER, WS_CHILD,
                        WS_OVERLAPPEDWINDOW, WS_VISIBLE,
                    },
                },
            },
        };

        struct Windows {
            previous: HWND,
            owned: Vec<HWND>,
        }
        impl Drop for Windows {
            fn drop(&mut self) {
                let restore = self.owned.contains(&unsafe { GetForegroundWindow() });
                for window in self.owned.iter().rev() {
                    let _ = unsafe { DestroyWindow(*window) };
                }
                if restore && !self.previous.is_invalid() {
                    let _ = unsafe { SetForegroundWindow(self.previous) };
                }
            }
        }
        fn pump() {
            let mut msg = MSG::default();
            while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
                let _ = unsafe { TranslateMessage(&msg) };
                unsafe { DispatchMessageW(&msg) };
            }
        }
        fn text(edit: HWND) -> String {
            let mut buffer = [0u16; 32];
            let count = unsafe { GetWindowTextW(edit, &mut buffer) }.max(0) as usize;
            String::from_utf16(&buffer[..count]).unwrap()
        }
        fn selected(edit: HWND) -> bool {
            let (mut first, mut last) = (0u32, 0u32);
            unsafe {
                SendMessageW(
                    edit,
                    0x00b0,
                    Some(WPARAM((&mut first as *mut u32) as usize)),
                    Some(LPARAM((&mut last as *mut u32) as isize)),
                )
            };
            (first, last) == (0, 4)
        }

        let mut windows = Windows {
            previous: unsafe { GetForegroundWindow() },
            owned: vec![],
        };
        let parent = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("OmniTrigger 脚本 API 实测"),
                WS_OVERLAPPEDWINDOW,
                100,
                100,
                500,
                160,
                None,
                None,
                None,
                None,
            )
        }
        .unwrap();
        windows.owned.push(parent);
        let edit = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("EDIT"),
                w!("abcd"),
                WS_CHILD | WS_VISIBLE | WS_BORDER | WINDOW_STYLE(0x0080),
                20,
                35,
                440,
                30,
                Some(parent),
                None,
                None,
                None,
            )
        }
        .unwrap();
        let decoy = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("失焦验证：不应收到输入"),
                WS_OVERLAPPEDWINDOW,
                120,
                120,
                500,
                160,
                None,
                None,
                None,
                None,
            )
        }
        .unwrap();
        windows.owned.push(decoy);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/actions");
        let catalog = PackageCatalog::load(&root);
        let mut registry = RuntimeRegistry::new();
        registry
            .register("node", Arc::new(NodeRuntime::default()))
            .unwrap();
        let service = ActionService::new(catalog, registry);
        let call: ActionCall = serde_json::from_str(include_str!(
            "../../../examples/actions/desktop-line-edit/call.json"
        ))
        .unwrap();
        let mut evidence = Vec::new();
        for mode in ["success", "focus_changed", "cancelled"] {
            unsafe { SetWindowTextW(edit, w!("abcd")) }.unwrap();
            let _ = unsafe { ShowWindow(parent, SW_SHOWNOACTIVATE) };
            let _ = unsafe { SetForegroundWindow(parent) };
            assert_eq!(
                unsafe { GetForegroundWindow() },
                parent,
                "未获取测试窗口前台，未提交动作"
            );
            let _ = unsafe { SetFocus(Some(edit)) };
            unsafe { SendMessageW(edit, 0x00b1, Some(WPARAM(4)), Some(LPARAM(4))) };
            let target = capture_shortcut_target().unwrap();
            crate::action_packages::submit(&service, 1, call.clone(), Some(target)).unwrap();
            let began = Instant::now();
            let mut altered = false;
            let outcome = loop {
                pump();
                if !altered && mode != "success" && selected(edit) {
                    if mode == "focus_changed" {
                        let _ = unsafe { ShowWindow(decoy, SW_SHOWNOACTIVATE) };
                        let _ = unsafe { SetForegroundWindow(decoy) };
                        assert_eq!(unsafe { GetForegroundWindow() }, decoy);
                    } else {
                        service.cancel_id(1);
                    }
                    altered = true;
                }
                if (mode != "focus_changed" || !altered)
                    && unsafe { GetForegroundWindow() } != parent
                {
                    service.cancel_all();
                    panic!("测试失焦，已请求取消");
                }
                if let Ok(event) = service.try_recv() {
                    if event.outcome.is_terminal() {
                        break event.outcome;
                    }
                }
                if began.elapsed() > Duration::from_secs(10) {
                    service.cancel_all();
                    panic!("脚本 API 实测超时");
                }
                thread::sleep(Duration::from_millis(1));
            };
            pump();
            match mode {
                "success" => {
                    assert!(
                        matches!(outcome, ActionOutcome::Succeeded { .. }),
                        "{outcome:?}"
                    );
                    assert_eq!(text(edit), "");
                }
                "focus_changed" => {
                    assert!(
                        matches!(&outcome, ActionOutcome::Failed { error } if error.code=="target_changed" && error.effect_state.as_deref()==Some("not_started")),
                        "{outcome:?}"
                    );
                    assert_eq!(text(edit), "abcd");
                }
                _ => {
                    assert_eq!(outcome, ActionOutcome::Cancelled);
                    assert_eq!(text(edit), "abcd");
                }
            }
            evidence.push(json!({"case":mode,"text":text(edit),"outcome":format!("{outcome:?}")}));
        }
        if let Some(path) = std::env::var_os("OMNITRIGGER_DESKTOP_SMOKE_REPORT") {
            std::fs::write(
                path,
                serde_json::to_vec_pretty(&json!({"cases":evidence})).unwrap(),
            )
            .unwrap();
        }
    }
}
