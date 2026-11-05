//! 显式交互验收：仅记录本程序注入标记的键码，不收集物理输入或窗口正文。
use crate::{
    desktop_host::DesktopHost,
    execution::{CommandExecutor, ExecutionOutcome},
    shortcut::ShortcutAction,
};
use omnitrigger_actions::{
    ActionCall, ActionOutcome, ActionService, NodeRuntime, PackageCatalog, RuntimeRegistry,
    ScriptHostRuntime,
};
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Input::KeyboardAndMouse::SetFocus,
            WindowsAndMessaging::{
                CallNextHookEx, CreateWindowExW, DestroyWindow, DispatchMessageW,
                GetForegroundWindow, GetWindowTextW, IsWindow, PeekMessageW, SendMessageW,
                SetForegroundWindow, SetWindowTextW, SetWindowsHookExW, ShowWindow,
                TranslateMessage, UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, MSG, PM_REMOVE,
                SW_SHOWNOACTIVATE, WH_KEYBOARD_LL, WINDOW_EX_STYLE, WINDOW_STYLE, WS_BORDER,
                WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
            },
        },
    },
};

#[derive(Clone, Debug)]
struct Key {
    vk: u32,
    scan: u32,
    up: bool,
    time: u32,
    at_us: u64,
    extended: bool,
}
static KEYS: Mutex<Vec<Key>> = Mutex::new(Vec::new());
static START: OnceLock<Instant> = OnceLock::new();
unsafe extern "system" fn observe(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0 {
        let key = unsafe { &*(l.0 as *const KBDLLHOOKSTRUCT) };
        if key.dwExtraInfo == 0x4f4d_534b {
            let mut keys = KEYS.lock().unwrap_or_else(|e| e.into_inner());
            if keys.len() < 128 {
                keys.push(Key {
                    vk: key.vkCode,
                    scan: key.scanCode,
                    up: key.flags.0 & 0x80 != 0,
                    time: key.time,
                    at_us: START.get().unwrap().elapsed().as_micros() as u64,
                    extended: key.flags.0 & 1 != 0,
                });
            }
        }
    }
    unsafe { CallNextHookEx(None, code, w, l) }
}
struct Hook(HHOOK);
impl Drop for Hook {
    fn drop(&mut self) {
        let _ = unsafe { UnhookWindowsHookEx(self.0) };
    }
}
struct Windows {
    main: HWND,
    other: HWND,
    previous: HWND,
}
impl Drop for Windows {
    fn drop(&mut self) {
        let foreground = unsafe { GetForegroundWindow() };
        let ours = foreground == self.main || foreground == self.other;
        let _ = unsafe { DestroyWindow(self.main) };
        let _ = unsafe { DestroyWindow(self.other) };
        if ours && !self.previous.is_invalid() {
            let _ = unsafe { SetForegroundWindow(self.previous) };
        }
    }
}
fn window(title: windows::core::PCWSTR) -> HWND {
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            title,
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
    .unwrap()
}
fn pump() {
    let mut message = MSG::default();
    while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
        let _ = unsafe { TranslateMessage(&message) };
        unsafe { DispatchMessageW(&message) };
    }
}
fn focus(window: HWND) {
    let _ = unsafe { ShowWindow(window, SW_SHOWNOACTIVATE) };
    let _ = unsafe { SetForegroundWindow(window) };
    pump();
    assert_eq!(
        unsafe { GetForegroundWindow() },
        window,
        "测试没有取得焦点，未发送输入"
    );
}
fn initial_focus(window: HWND) {
    if let Some(path) = std::env::var_os("OMNITRIGGER_KEY_SMOKE_READY") {
        // 显式交互验收可由外部控制器激活这一自建窗口；不绕过系统前台锁，
        // 等待期间只泵消息，不采集或注入任何输入。
        let _ = unsafe { ShowWindow(window, SW_SHOWNOACTIVATE) };
        std::fs::write(
            path,
            serde_json::to_vec(&json!({
                "pid": std::process::id(),
                "window_id": window.0 as usize,
            }))
            .unwrap(),
        )
        .unwrap();
        let start = Instant::now();
        while unsafe { GetForegroundWindow() } != window {
            pump();
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "等待专用窗口激活超时，未发送输入"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
    focus(window);
}
fn keys() -> Vec<Key> {
    KEYS.lock().unwrap().clone()
}
fn reset() {
    KEYS.lock().unwrap().clear();
}
fn settle() {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(40) {
        pump();
        thread::sleep(Duration::from_millis(1));
    }
}
fn verify(expected: &[(u32, bool)], gaps: &[(usize, usize, u32)]) -> Value {
    settle();
    let observed = keys();
    assert_eq!(
        observed
            .iter()
            .map(|key| (key.vk, key.up))
            .collect::<Vec<_>>(),
        expected
    );
    for &(from, to, minimum) in gaps {
        assert!(
            observed[to].at_us.saturating_sub(observed[from].at_us) >= u64::from(minimum) * 1000,
            "按下间隔未达到最小值：{from}->{to}, {observed:?}"
        );
    }
    json!({"keys":observed.iter().map(|key|json!({"vk":key.vk,"scan":key.scan,"up":key.up,"time":key.time,"at_us":key.at_us,"extended":key.extended})).collect::<Vec<_>>(),"verified":true})
}
struct Running<'a>(&'a ActionService);
impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.cancel_all();
        let start = Instant::now();
        while self.0.is_running(1) && start.elapsed() < Duration::from_secs(5) {
            pump();
            let _ = self.0.try_recv();
            thread::sleep(Duration::from_millis(1));
        }
    }
}

#[test]
#[ignore = "激活专用测试窗口，观察本程序注入的键码、延迟、失焦与取消收尾"]
fn interactive_desktop_key_sequence_smoke() {
    let windows = Windows {
        main: window(w!("OmniTrigger 连续按键 API 实测")),
        other: window(w!("OmniTrigger 失焦收尾实测")),
        previous: unsafe { GetForegroundWindow() },
    };
    KEYS.lock().unwrap().reserve(128);
    START.get_or_init(Instant::now);
    let module = unsafe { GetModuleHandleW(None) }.unwrap();
    let _hook = Hook(
        unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(observe), Some(HINSTANCE(module.0)), 0) }
            .unwrap(),
    );
    initial_focus(windows.main);
    let executor = CommandExecutor::new();
    reset();
    let raw = omnitrigger_windows::capture_shortcut_target().unwrap();
    let process = omnitrigger_windows::ScriptTarget::capture(raw)
        .unwrap()
        .info()
        .unwrap()
        .process_name;
    let mut action = ShortcutAction::new(vec![163, 165, 161, 135]);
    action.timing.process_overrides.insert(process, 10);
    executor.submit_shortcut_to(1, action, raw).unwrap();
    let began = Instant::now();
    loop {
        pump();
        assert_eq!(
            unsafe { GetForegroundWindow() },
            windows.main,
            "原生按键实测失焦"
        );
        if let Ok(event) = executor.try_recv() {
            assert!(
                matches!(event.outcome, ExecutionOutcome::ShortcutSent),
                "{:?}",
                event.outcome
            );
            break;
        }
        assert!(began.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(1));
    }
    let mut reports = vec![
        json!({"case":"native-process-timing","evidence":verify(&[(163,false),(165,false),(161,false),(135,false),(135,true),(161,true),(165,true),(163,true)],&[(2,3,8),(3,4,8)])}),
    ];
    let captured = keys();
    assert!(captured[0].extended && captured[1].extended, "{captured:?}");
    // 低层回调中的扩展位由系统归一化，不等同于提交的 KEYEVENTF_EXTENDEDKEY。
    assert_eq!(captured[2].scan, 0x36, "右 Shift 必须保留自身扫描码");
    let mut registry = RuntimeRegistry::new();
    registry
        .register("node", Arc::new(NodeRuntime::default()))
        .unwrap();
    let mut line_recipes = vec!["recipe"];
    if let Some(program) = std::env::var_os("OMNITRIGGER_TEST_SCRIPT_HOST") {
        registry
            .register("script", Arc::new(ScriptHostRuntime::new(program)))
            .unwrap();
        line_recipes.push("recipe-script");
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/actions");
    let service = ActionService::new(PackageCatalog::load(&root), registry);
    let _running = Running(&service);
    for (name, normal, delay, mode, modifier) in [
        ("sequence", vec![75, 67], 10, 0, 163),
        ("left-control-sequence", vec![75, 85], 10, 0, 162),
        ("source-uncomment-default-timing", vec![75, 85], 0, 0, 162),
        ("source-comment-default-timing", vec![75, 67], 0, 0, 162),
        ("double-key", vec![77, 77], 0, 0, 163),
        ("cancel-during-key", vec![75, 67], 200, 1, 163),
        ("focus-during-key", vec![75, 67], 200, 2, 163),
    ] {
        assert!(
            [windows.main, windows.other].contains(&unsafe { GetForegroundWindow() }),
            "用户已切换到其他窗口，停止实测"
        );
        focus(windows.main);
        reset();
        let raw = omnitrigger_windows::capture_shortcut_target().unwrap();
        let mut input = json!({"modifiers":[modifier],"keys":normal});
        if delay != 0 {
            input["keyDownDelayMs"] = delay.into();
        }
        service
            .submit_with_host(
                1,
                ActionCall {
                    package_id: "examples.keyboard-sequence".into(),
                    action_id: "sequence".into(),
                    action_version: 1,
                    input,
                },
                Some(Arc::new(DesktopHost::capture(Some(raw)))),
            )
            .unwrap();
        let began = Instant::now();
        let mut changed = false;
        let outcome = loop {
            pump();
            if mode != 0 && !changed && keys().iter().any(|key| key.vk == 75 && !key.up) {
                if mode == 1 {
                    service.cancel_id(1);
                } else {
                    focus(windows.other);
                }
                changed = true;
            }
            let expected_foreground = if mode == 2 && changed {
                windows.other
            } else {
                windows.main
            };
            if unsafe { GetForegroundWindow() } != expected_foreground {
                service.cancel_all();
                panic!("测试切换到了未知窗口，已取消输入");
            }
            if let Ok(event) = service.try_recv() {
                if event.outcome.is_terminal() {
                    break event.outcome;
                }
            }
            if began.elapsed() > Duration::from_secs(8) {
                service.cancel_all();
                panic!("连续按键API实测超时");
            }
            thread::sleep(Duration::from_millis(1));
        };
        let expected = if mode == 0 {
            vec![
                (modifier, false),
                (normal[0], false),
                (normal[0], true),
                (normal[1], false),
                (normal[1], true),
                (modifier, true),
            ]
        } else {
            assert!(changed);
            vec![(163, false), (75, false), (75, true), (163, true)]
        };
        if mode == 0 {
            assert!(
                matches!(outcome, ActionOutcome::Succeeded { .. }),
                "{outcome:?}"
            );
        } else {
            assert!(
                !matches!(outcome, ActionOutcome::Succeeded { .. }),
                "取消或失焦后不能成功"
            );
        }
        let gaps = if mode == 0 && delay > 0 {
            vec![(0, 1, 8), (1, 2, 8), (3, 4, 8)]
        } else {
            vec![]
        };
        reports.push(json!({"case":name,"evidence":verify(&expected,&gaps),"outcome":format!("{outcome:?}")}));
    }
    for (name, modifier, first, last, gap) in [
        ("rename-series", 162, 82, 82, 30),
        ("fold-series", 162, 77, 76, 80),
    ] {
        assert!([windows.main, windows.other].contains(&unsafe { GetForegroundWindow() }));
        focus(windows.main);
        reset();
        let raw = omnitrigger_windows::capture_shortcut_target().unwrap();
        service.submit_with_host(1,ActionCall{package_id:"examples.keyboard-sequence".into(),action_id:"shortcuts".into(),action_version:1,
            input:json!({"keyDownDelayMs":0,"steps":[{"keys":[modifier,first],"delayAfterMs":gap},{"keys":[modifier,last]}]})},Some(Arc::new(DesktopHost::capture(Some(raw))))).unwrap();
        let began = Instant::now();
        loop {
            pump();
            if unsafe { GetForegroundWindow() } != windows.main {
                service.cancel_all();
                panic!("独立快捷键实测失焦");
            }
            if let Ok(event) = service.try_recv() {
                if event.outcome.is_terminal() {
                    assert!(
                        matches!(event.outcome, ActionOutcome::Succeeded { .. }),
                        "{:?}",
                        event.outcome
                    );
                    break;
                }
            }
            if began.elapsed() > Duration::from_secs(5) {
                service.cancel_all();
                panic!("独立快捷键实测超时");
            }
            thread::sleep(Duration::from_millis(1));
        }
        let expected = [
            (modifier, false),
            (first, false),
            (first, true),
            (modifier, true),
            (modifier, false),
            (last, false),
            (last, true),
            (modifier, true),
        ];
        reports.push(json!({"case":name,"evidence":verify(&expected,&[(3,4,gap-2)])}));
    }
    if let Some(path) = std::env::var_os("OMNITRIGGER_KEY_RECIPE_FIXTURE") {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Case {
            name: String,
            call: ActionCall,
            expected: Vec<(u32, bool)>,
            gaps: Vec<(usize, usize, u32)>,
        }
        let file = std::fs::File::open(path).unwrap();
        assert!(file.metadata().unwrap().len() <= 1024 * 1024);
        let cases: Vec<Case> = serde_json::from_reader(file).unwrap();
        assert!(!cases.is_empty() && cases.len() <= 32);
        for case in cases {
            assert_eq!(case.call.package_id, "examples.keyboard-sequence");
            assert_eq!(case.call.action_id, "recipe");
            assert!(!case.expected.is_empty() && case.expected.len() <= 64);
            focus(windows.main);
            reset();
            service
                .submit_with_host(
                    1,
                    case.call,
                    Some(Arc::new(DesktopHost::capture(Some(
                        omnitrigger_windows::capture_shortcut_target().unwrap(),
                    )))),
                )
                .unwrap();
            let start = Instant::now();
            loop {
                pump();
                assert_eq!(
                    unsafe { GetForegroundWindow() },
                    windows.main,
                    "外部配方实测失焦"
                );
                if let Ok(event) = service.try_recv() {
                    if event.outcome.is_terminal() {
                        assert!(
                            matches!(event.outcome, ActionOutcome::Succeeded { .. }),
                            "{event:?}"
                        );
                        break;
                    }
                }
                assert!(
                    start.elapsed() < Duration::from_secs(10),
                    "外部配方实测超时"
                );
                thread::sleep(Duration::from_millis(1));
            }
            reports.push(json!({"case":case.name,"evidence":verify(&case.expected,&case.gaps)}));
        }
    }
    if let Some(path) = std::env::var_os("OMNITRIGGER_SHORTCUT_FIXTURE") {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Case {
            name: String,
            action: ShortcutAction,
            expected: Vec<(u32, bool)>,
            gaps: Vec<(usize, usize, u32)>,
        }
        let file = std::fs::File::open(path).unwrap();
        assert!(file.metadata().unwrap().len() <= 1024 * 1024);
        let cases: Vec<Case> = serde_json::from_reader(file).unwrap();
        assert!(!cases.is_empty() && cases.len() <= 32);
        for case in cases {
            assert!(!case.expected.is_empty() && case.expected.len() <= 64);
            case.action.validate().unwrap();
            for runtime in ["native", "node"] {
                assert!([windows.main, windows.other].contains(&unsafe { GetForegroundWindow() }));
                focus(windows.main);
                reset();
                let raw = omnitrigger_windows::capture_shortcut_target().unwrap();
                let process = omnitrigger_windows::ScriptTarget::capture(raw)
                    .unwrap()
                    .info()
                    .unwrap()
                    .process_name;
                let delay = case.action.timing.resolve(&process).unwrap();
                if runtime == "native" {
                    executor
                        .submit_shortcut_to(1, case.action.clone(), raw)
                        .unwrap();
                } else {
                    service
                        .submit_with_host(
                            1,
                            ActionCall {
                                package_id: "examples.shortcut".into(),
                                action_id: "send".into(),
                                action_version: 1,
                                input: serde_json::to_value(&case.action).unwrap(),
                            },
                            Some(Arc::new(DesktopHost::capture(Some(raw)))),
                        )
                        .unwrap();
                }
                let began = Instant::now();
                loop {
                    pump();
                    assert_eq!(
                        unsafe { GetForegroundWindow() },
                        windows.main,
                        "快捷键实测失焦"
                    );
                    let done = if runtime == "native" {
                        executor
                            .try_recv()
                            .ok()
                            .map(|event| {
                                assert!(
                                    matches!(event.outcome, ExecutionOutcome::ShortcutSent),
                                    "{event:?}"
                                );
                                true
                            })
                            .unwrap_or(false)
                    } else if let Ok(event) = service.try_recv() {
                        if event.outcome.is_terminal() {
                            assert!(
                                matches!(event.outcome, ActionOutcome::Succeeded { .. }),
                                "{event:?}"
                            );
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    if done {
                        break;
                    }
                    assert!(began.elapsed() < Duration::from_secs(10), "快捷键实测超时");
                    thread::sleep(Duration::from_millis(1));
                }
                reports.push(json!({"case":case.name,"runtime":runtime,"resolved_delay_ms":delay,"evidence":verify(&case.expected,&case.gaps)}));
            }
        }
    }
    // 验证同一输入配方在自建多行输入框中的实际选区与行边界行为。
    let edit = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("EDIT"),
            w!(""),
            WS_CHILD | WS_VISIBLE | WS_BORDER | WINDOW_STYLE(0x0004),
            10,
            30,
            460,
            100,
            Some(windows.main),
            None,
            None,
            None,
        )
    }
    .unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/actions");
    let call: ActionCall = serde_json::from_slice(
        &std::fs::read(root.join("keyboard-sequence/delete-line-start.json")).unwrap(),
    )
    .unwrap();
    for action_id in line_recipes {
        for delay in [0, 10] {
            for (position, expected) in [
                (9, "head\r\nDEF\r\ntail"),
                (12, "head\r\n\r\ntail"),
                (6, "headabcDEF\r\ntail"),
            ] {
                focus(windows.main);
                let _ = unsafe { SetFocus(Some(edit)) };
                let before = "head\r\nabcDEF\r\ntail"
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect::<Vec<_>>();
                unsafe {
                    SetWindowTextW(edit, PCWSTR(before.as_ptr())).unwrap();
                    SendMessageW(
                        edit,
                        0x00b1,
                        Some(WPARAM(position)),
                        Some(LPARAM(position as isize)),
                    );
                }
                reset();
                let mut call = call.clone();
                call.action_id = action_id.into();
                call.input["keyDownDelayMs"] = delay.into();
                service
                    .submit_with_host(
                        1,
                        call,
                        Some(Arc::new(DesktopHost::capture(Some(
                            omnitrigger_windows::capture_shortcut_target().unwrap(),
                        )))),
                    )
                    .unwrap();
                let start = Instant::now();
                loop {
                    pump();
                    assert_eq!(
                        unsafe { GetForegroundWindow() },
                        windows.main,
                        "行编辑实测失焦"
                    );
                    if let Ok(event) = service.try_recv() {
                        if event.outcome.is_terminal() {
                            assert!(
                                matches!(event.outcome, ActionOutcome::Succeeded { .. }),
                                "{event:?}"
                            );
                            break;
                        }
                    }
                    assert!(start.elapsed() < Duration::from_secs(10), "行编辑实测超时");
                    thread::sleep(Duration::from_millis(1));
                }
                let gaps = if delay == 0 {
                    vec![(3, 4, 78)]
                } else {
                    vec![(0, 1, 8), (1, 2, 8), (3, 4, 88), (4, 5, 8)]
                };
                let key_evidence = verify(
                    &[
                        (160, false),
                        (36, false),
                        (36, true),
                        (160, true),
                        (8, false),
                        (8, true),
                    ],
                    &gaps,
                );
                let mut buffer = [0u16; 128];
                let count = unsafe { GetWindowTextW(edit, &mut buffer) }.max(0) as usize;
                let observed = String::from_utf16(&buffer[..count]).unwrap();
                assert_eq!(observed, expected);
                reports.push(json!({"case":"delete-line-start","action_id":action_id,"key_down_delay_ms":delay,"cursor":position,"expected":expected,"observed":observed,"evidence":key_evidence}));
            }
        }
    }
    let report = json!({"cases":reports,"clipboard_accessed":false,"physical_keys_recorded":false,"scope":"tagged_input_and_owned_edit_not_physical_trigger_or_original_target_application_evidence"});
    if let Some(path) = std::env::var_os("OMNITRIGGER_KEY_SMOKE_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    println!(
        "原生、脚本按键和自建输入框行编辑共{}项实测通过；未访问剪贴板或记录物理键盘输入。",
        reports.len()
    );
}

#[test]
#[ignore = "激活自建窗口，经真实轻量脚本验证关闭效果；需要 OMNITRIGGER_TEST_SCRIPT_HOST"]
fn interactive_close_recipe_smoke() {
    let program = std::env::var_os("OMNITRIGGER_TEST_SCRIPT_HOST").expect("需要真实轻量宿主路径");
    let windows = Windows {
        main: window(w!("OmniTrigger 关闭动作效果实测")),
        other: window(w!("OmniTrigger 关闭动作备用窗口")),
        previous: unsafe { GetForegroundWindow() },
    };
    initial_focus(windows.main);
    let raw = omnitrigger_windows::capture_shortcut_target().unwrap();
    let process = omnitrigger_windows::ScriptTarget::capture(raw)
        .unwrap()
        .info()
        .unwrap()
        .process_name;
    assert_ne!(process, "Acrobat.exe", "本例验证默认关闭窗口分支");
    let mut registry = RuntimeRegistry::new();
    registry
        .register(
            "script",
            Arc::new(ScriptHostRuntime::new(program).with_timeout(Duration::from_secs(5))),
        )
        .unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/actions");
    let call: ActionCall =
        serde_json::from_slice(&std::fs::read(root.join("common-desktop/call.json")).unwrap())
            .unwrap();
    assert_eq!(call.input["ctrlWProcesses"], json!(["Acrobat"]));
    let service = ActionService::new(PackageCatalog::load(&root), registry);
    let _running = Running(&service);
    service
        .submit_with_host(1, call, Some(Arc::new(DesktopHost::capture(Some(raw)))))
        .unwrap();
    let start = Instant::now();
    let mut outcome = None;
    loop {
        pump();
        let closed = !unsafe { IsWindow(Some(windows.main)) }.as_bool();
        if !closed && unsafe { GetForegroundWindow() } != windows.main {
            service.cancel_all();
            panic!("关闭效果验收失焦，已取消输入");
        }
        if let Ok(event) = service.try_recv() {
            if event.outcome.is_terminal() {
                assert!(
                    matches!(event.outcome, ActionOutcome::Succeeded { .. }),
                    "{:?}",
                    event.outcome
                );
                if let ActionOutcome::Succeeded { output } = event.outcome {
                    assert_eq!(output["operation"], "alt_f4");
                    assert_eq!(output["submitted"], true);
                    outcome = Some(output);
                }
            }
        }
        if closed && outcome.is_some() {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(8), "自建窗口未关闭");
        thread::sleep(Duration::from_millis(1));
    }
    let report = json!({
        "case": "close-by-process-alt-f4",
        "window_closed": true,
        "outcome": outcome,
        "clipboard_accessed": false,
        "scope": "owned_window_not_original_application_or_ctrl_w_branch",
    });
    if let Some(path) = std::env::var_os("OMNITRIGGER_KEY_SMOKE_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    println!("真实轻量脚本已关闭本轮自建窗口，未访问剪贴板。");
}
