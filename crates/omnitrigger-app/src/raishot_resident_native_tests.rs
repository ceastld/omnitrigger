//! 真实 Win32 边界；只在专用测试进程的 CREATE_ONLY 非交互窗口站运行。
//! 固定类名接收器仅统计消息，不加载 RaiShot，不截图，也不访问剪贴板。

use super::execute;
use omnitrigger_actions::CancellationToken;
use omnitrigger_windows::{
    find_raishot_resident, request_raishot_resident_region, RAISHOT_REGION_MESSAGE,
    RAISHOT_RESIDENT_CLASS,
};
use serde_json::{json, Value};
use std::{
    sync::atomic::{AtomicIsize, AtomicUsize, Ordering},
    time::Instant,
};
use windows::{
    core::w,
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
        System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, IsWindowVisible,
            PeekMessageW, RegisterClassW, UnregisterClassW, MSG, PM_REMOVE, WINDOW_EX_STYLE,
            WINDOW_STYLE, WNDCLASSW,
        },
    },
};

// 复用既有 helper 的原文件；它只管理站/桌面，没有剪贴板读写。
#[path = "../../omnitrigger-windows/tests/support/private_clipboard.rs"]
mod private_station;
use private_station::PrivateClipboard;

const FIND: &str = "raishot.resident.find";
const REGION: &str = "raishot.resident.region";
static REQUESTS: AtomicUsize = AtomicUsize::new(0);
static LAST_WPARAM: AtomicUsize = AtomicUsize::new(usize::MAX);
static LAST_LPARAM: AtomicIsize = AtomicIsize::new(isize::MAX);

unsafe extern "system" fn receiver(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == RAISHOT_REGION_MESSAGE {
        LAST_WPARAM.store(wparam.0, Ordering::SeqCst);
        LAST_LPARAM.store(lparam.0, Ordering::SeqCst);
        REQUESTS.fetch_add(1, Ordering::SeqCst);
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

struct OwnedWindow<'a> {
    scope: &'a PrivateClipboard,
    hwnd: Option<HWND>,
    instance: HINSTANCE,
    registered: bool,
}

impl<'a> OwnedWindow<'a> {
    fn create(scope: &'a PrivateClipboard) -> Self {
        scope.verify();
        let instance: HINSTANCE = unsafe { GetModuleHandleW(None) }.unwrap().into();
        let class = WNDCLASSW {
            lpfnWndProc: Some(receiver),
            hInstance: instance,
            lpszClassName: w!("RustShot.MVP.Window"),
            ..Default::default()
        };
        assert_ne!(unsafe { RegisterClassW(&class) }, 0);
        let mut owner = Self {
            scope,
            hwnd: None,
            instance,
            registered: true,
        };
        scope.verify();
        // FindWindowW 不查找 HWND_MESSAGE；使用无 WS_VISIBLE 的零尺寸顶层窗口。
        // 所在窗口站已证明不可交互，从不调用 ShowWindow 或前台切换接口。
        owner.hwnd = Some(
            unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("RustShot.MVP.Window"),
                    w!("OmniTrigger isolated resident protocol receiver"),
                    WINDOW_STYLE(0),
                    0,
                    0,
                    0,
                    0,
                    None,
                    None,
                    Some(instance),
                    None,
                )
            }
            .expect("只允许在已验证的私有窗口站创建接收器"),
        );
        assert!(!unsafe { IsWindowVisible(owner.handle()) }.as_bool());
        owner
    }

    fn handle(&self) -> HWND {
        self.hwnd.expect("接收器已关闭")
    }

    fn pump(&self) {
        self.scope.verify();
        for _ in 0..64 {
            let mut message = MSG::default();
            if !unsafe { PeekMessageW(&mut message, Some(self.handle()), 0, 0, PM_REMOVE) }
                .as_bool()
            {
                return;
            }
            assert_eq!(message.hwnd, self.handle());
            unsafe { DispatchMessageW(&message) };
        }
        panic!("私有接收器消息数量超限");
    }

    fn assert_no_extra_request(&self) {
        self.pump();
        assert_eq!(REQUESTS.load(Ordering::SeqCst), 1);
        assert!(!unsafe { IsWindowVisible(self.handle()) }.as_bool());
    }

    fn close(&mut self) {
        self.scope.verify();
        if let Some(hwnd) = self.hwnd {
            unsafe { DestroyWindow(hwnd) }.expect("应销毁自有接收器");
            self.hwnd = None;
        }
        if self.registered {
            unsafe { UnregisterClassW(w!("RustShot.MVP.Window"), Some(self.instance)) }
                .expect("应注销本进程注册的类");
            self.registered = false;
        }
    }
}

impl Drop for OwnedWindow<'_> {
    fn drop(&mut self) {
        self.scope.verify();
        if let Some(hwnd) = self.hwnd {
            let _ = unsafe { DestroyWindow(hwnd) };
        }
        if self.registered {
            let _ = unsafe { UnregisterClassW(w!("RustShot.MVP.Window"), Some(self.instance)) };
        }
    }
}

fn find_owned(
    scope: &PrivateClipboard,
    owner: &OwnedWindow<'_>,
    token: &CancellationToken,
) -> Value {
    scope.verify();
    let snapshot = execute(FIND, json!({}), token, None).unwrap();
    scope.verify();
    let expected_handle = (owner.handle().0 as usize as u64).to_string();
    assert_eq!(
        snapshot["window_handle"].as_str(),
        Some(expected_handle.as_str())
    );
    assert_eq!(snapshot["process_id"], std::process::id());
    assert_eq!(snapshot["thread_id"], unsafe { GetCurrentThreadId() });
    assert!(
        snapshot["process_created_file_time"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > 0
    );
    snapshot
}

fn rejected(
    scope: &PrivateClipboard,
    method: &str,
    params: Value,
    token: &CancellationToken,
    deadline: Option<Instant>,
    code: &str,
) {
    scope.verify();
    let error = execute(method, params, token, deadline).expect_err("必须拒绝且不投递");
    assert_eq!(error.code, code);
    assert_eq!(error.method.as_deref(), Some(method));
    assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    scope.verify();
}

fn require_dedicated_test_process() {
    let expected = format!(
        "{}::native_raishot_protocol_in_private_station",
        module_path!().split_once("::").unwrap().1
    );
    let args: Vec<String> = std::env::args().skip(1).collect();
    assert!(args.iter().any(|value| value == "--exact"));
    assert!(args.iter().any(|value| value == &expected));
    assert!(args.iter().any(|value| value == "--ignored"));
    assert!(
        args.iter().any(|value| value == "--test-threads=1")
            || args
                .windows(2)
                .any(|pair| pair[0] == "--test-threads" && pair[1] == "1")
    );
    assert_eq!(
        std::env::var("OMNITRIGGER_RAISHOT_PRIVATE_STATION_TEST").as_deref(),
        Ok("1")
    );
}

#[test]
#[ignore = "仅显式指定的独立测试进程、exact 单用例和单线程；禁止回退默认窗口站"]
fn native_raishot_protocol_in_private_station() {
    require_dedicated_test_process();
    // 在任何 find / 注册类 / 创建窗口之前，先完成 CREATE_ONLY、切站与不可交互验证。
    let scope = PrivateClipboard::create();
    scope.verify();
    let active = CancellationToken::new();
    let mut cases = Vec::new();
    assert_eq!(
        execute(FIND, json!({}), &active, None).unwrap(),
        Value::Null
    );
    cases.push("empty_private_station_does_not_find_default_station_resident");
    assert_eq!(RAISHOT_RESIDENT_CLASS, "RustShot.MVP.Window");
    assert_eq!(RAISHOT_REGION_MESSAGE, 32771);
    REQUESTS.store(0, Ordering::SeqCst);
    LAST_WPARAM.store(usize::MAX, Ordering::SeqCst);
    LAST_LPARAM.store(isize::MAX, Ordering::SeqCst);
    let mut owner = OwnedWindow::create(&scope);
    let snapshot = find_owned(&scope, &owner, &active);
    cases.push("hidden_owned_window_identity");
    scope.verify();
    let reply = execute(REGION, json!({"resident":snapshot}), &active, None).unwrap();
    assert_eq!(reply, json!({"queued":true,"screenshot_completed":false}));
    // PostMessage 只入队；当前线程未泵消息前，接收器尚未执行。
    assert_eq!(REQUESTS.load(Ordering::SeqCst), 0);
    owner.pump();
    assert_eq!(REQUESTS.load(Ordering::SeqCst), 1);
    assert_eq!(LAST_WPARAM.load(Ordering::SeqCst), 0);
    assert_eq!(LAST_LPARAM.load(Ordering::SeqCst), 0);
    owner.assert_no_extra_request();
    cases.push("one_32771_message_with_two_zero_parameters");

    for field in ["process_id", "thread_id", "process_created_file_time"] {
        let mut bad = find_owned(&scope, &owner, &active);
        bad[field] = if field == "process_created_file_time" {
            json!((bad[field].as_str().unwrap().parse::<u64>().unwrap() + 1).to_string())
        } else {
            json!(bad[field].as_u64().unwrap() + 1)
        };
        rejected(
            &scope,
            REGION,
            json!({"resident":bad}),
            &active,
            None,
            "raishot_resident_changed",
        );
        owner.assert_no_extra_request();
        cases.push(field);
    }
    let mut bad = find_owned(&scope, &owner, &active);
    bad["window_handle"] = json!("0");
    rejected(
        &scope,
        REGION,
        json!({"resident":bad}),
        &active,
        None,
        "invalid_input",
    );
    owner.assert_no_extra_request();
    cases.push("zero_handle_rejected");

    for expires in [0, super::now_ms().unwrap() + 60000] {
        let mut bad = find_owned(&scope, &owner, &active);
        bad["expires_at_unix_ms"] = json!(expires);
        rejected(
            &scope,
            REGION,
            json!({"resident":bad}),
            &active,
            None,
            "raishot_resident_expired",
        );
        owner.assert_no_extra_request();
    }
    cases.push("expired_and_overlong_lease_rejected");
    let snapshot = find_owned(&scope, &owner, &active);
    rejected(
        &scope,
        FIND,
        json!({"class":"untrusted"}),
        &active,
        None,
        "invalid_input",
    );
    rejected(
        &scope,
        REGION,
        json!({"resident":snapshot,"extra":true}),
        &active,
        None,
        "invalid_input",
    );
    owner.assert_no_extra_request();
    cases.push("fixed_schema_only");

    let cancelled = CancellationToken::new();
    cancelled.cancel();
    rejected(&scope, FIND, json!({}), &cancelled, None, "cancelled");
    rejected(
        &scope,
        REGION,
        json!({"resident":find_owned(&scope,&owner,&active)}),
        &cancelled,
        None,
        "cancelled",
    );
    owner.assert_no_extra_request();
    cases.push("cancelled_host_find_and_region_rejected");
    rejected(
        &scope,
        REGION,
        json!({"resident":find_owned(&scope,&owner,&active)}),
        &active,
        Some(Instant::now()),
        "timeout",
    );
    owner.assert_no_extra_request();
    cases.push("expired_host_deadline_rejected");

    scope.verify();
    let native = find_raishot_resident().unwrap().unwrap();
    assert_eq!(native.window_handle, owner.handle().0 as usize as u64);
    for reject_at in [1, 2] {
        let mut checkpoints = 0;
        scope.verify();
        let result = request_raishot_resident_region(&native, || {
            scope.verify();
            checkpoints += 1;
            if checkpoints == reject_at {
                Err("cancelled")
            } else {
                Ok(())
            }
        });
        assert_eq!(result, Err("cancelled"));
        assert_eq!(checkpoints, reject_at);
        owner.assert_no_extra_request();
    }
    cases.push("platform_cancellation_before_identity_and_before_post");

    let mut stale = find_owned(&scope, &owner, &active);
    owner.close();
    scope.verify();
    // 只刷新测试 lease，确保这里检验失效 HWND，而不是先命中过期分支。
    stale["expires_at_unix_ms"] = json!(super::now_ms().unwrap() + 5000);
    rejected(
        &scope,
        REGION,
        json!({"resident":stale}),
        &active,
        None,
        "raishot_resident_changed",
    );
    assert_eq!(
        execute(FIND, json!({}), &active, None).unwrap(),
        Value::Null
    );
    assert_eq!(REQUESTS.load(Ordering::SeqCst), 1);
    scope.verify();
    cases.push("destroyed_identity_rejected_and_private_find_empty");
    println!(
        "RAISHOT_PRIVATE_NATIVE_RESULT {}",
        json!({
            "cases":cases,"accepted_posts":1,"received_messages":REQUESTS.load(Ordering::SeqCst),
            "message":32771,"wparam":LAST_WPARAM.load(Ordering::SeqCst),"lparam":LAST_LPARAM.load(Ordering::SeqCst),
            "owned_window_destroyed":true,"owned_class_unregistered":true,
            "visible_windows_created":0,"foreground_calls":0,"clipboard_calls":0,
            "real_raishot_started":false,"screenshot_completed":false
        })
    );
    // owner 先于 scope 析构；helper 按原约定恢复线程桌面/进程窗口站并关闭自有句柄。
}
