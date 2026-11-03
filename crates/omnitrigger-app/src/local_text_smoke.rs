//! 显式交互测试：只向自建输入框执行单步文本动作，不读取或改写剪贴板。
use crate::{
    local_action::{LocalAction, LocalStep},
    local_executor::{LocalExecutionOutcome, LocalExecutor},
    settings::ActionSpec,
};
use serde::Deserialize;
use serde_json::json;
use std::{
    cell::Cell,
    thread,
    time::{Duration, Instant},
};
use windows::{
    core::w,
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        System::DataExchange::GetClipboardSequenceNumber,
        UI::{
            Input::KeyboardAndMouse::SetFocus,
            WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, DispatchMessageW, GetForegroundWindow,
                GetWindowTextW, PeekMessageW, SendMessageW, SetForegroundWindow, SetWindowTextW,
                ShowWindow, TranslateMessage, MSG, PM_REMOVE, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE,
                WS_BORDER, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
            },
        },
    },
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: u64,
    expected: String,
    action: ActionSpec,
}

fn pump() {
    let mut message = MSG::default();
    while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
        let _ = unsafe { TranslateMessage(&message) };
        unsafe { DispatchMessageW(&message) };
    }
}
fn text(edit: HWND) -> String {
    let mut buffer = [0u16; 256];
    let count = unsafe { GetWindowTextW(edit, &mut buffer) }.max(0) as usize;
    String::from_utf16(&buffer[..count]).unwrap()
}
struct Window(HWND, HWND);
impl Drop for Window {
    fn drop(&mut self) {
        let ours = unsafe { GetForegroundWindow() } == self.0;
        let _ = unsafe { DestroyWindow(self.0) };
        if ours && !self.1.is_invalid() {
            let _ = unsafe { SetForegroundWindow(self.1) };
        }
    }
}
struct Running<'a> {
    executor: &'a LocalExecutor,
    id: Cell<u64>,
}
impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.executor.cancel_all();
        let started = Instant::now();
        while self.executor.is_running(self.id.get()) && started.elapsed() < Duration::from_secs(5)
        {
            pump();
            let _ = self.executor.try_recv();
            thread::sleep(Duration::from_millis(1));
        }
    }
}

#[test]
#[ignore = "激活自建输入框并验证真实本地文本步骤；不访问剪贴板内容"]
fn interactive_local_literal_text_smoke() {
    let cases: Vec<Case> = if let Some(path) = std::env::var_os("OMNITRIGGER_LITERAL_TEXT_FIXTURE")
    {
        let file = std::fs::File::open(path).unwrap();
        assert!(file.metadata().unwrap().len() <= 1024 * 1024);
        serde_json::from_reader(file).unwrap()
    } else {
        vec![Case {
            id: 1,
            expected: "${literal} $$ 文😀 \\".into(),
            action: ActionSpec::Local(LocalAction {
                version: 1,
                steps: vec![LocalStep::InputText {
                    text: "$${literal} $$$$ 文😀 \\".into(),
                }],
            }),
        }]
    };
    assert!(!cases.is_empty() && cases.len() <= 64);
    for case in &cases {
        assert!(case.expected.encode_utf16().count() <= 64);
        let ActionSpec::Local(action) = &case.action else {
            panic!("只接受本地文本动作")
        };
        assert!(matches!(
            action.steps.as_slice(),
            [LocalStep::InputText { .. }]
        ));
        action.validate().unwrap();
    }
    let window = Window(
        unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("OmniTrigger 字面文本实测"),
                WS_OVERLAPPEDWINDOW,
                100,
                100,
                520,
                200,
                None,
                None,
                None,
                None,
            )
        }
        .unwrap(),
        unsafe { GetForegroundWindow() },
    );
    let edit = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("EDIT"),
            w!(""),
            WS_CHILD | WS_VISIBLE | WS_BORDER,
            20,
            35,
            460,
            80,
            Some(window.0),
            None,
            None,
            None,
        )
    }
    .unwrap();
    let _ = unsafe { ShowWindow(window.0, SW_SHOWNOACTIVATE) };
    let _ = unsafe { SetForegroundWindow(window.0) };
    assert_eq!(
        unsafe { GetForegroundWindow() },
        window.0,
        "没有取得自建窗口焦点，未提交输入"
    );
    let _ = unsafe { SetFocus(Some(edit)) };
    let target = omnitrigger_windows::capture_shortcut_target().unwrap();
    let executor = LocalExecutor::new();
    let running = Running {
        executor: &executor,
        id: Cell::new(0),
    };
    let mut evidence = Vec::new();
    for case in cases {
        assert_eq!(
            unsafe { GetForegroundWindow() },
            window.0,
            "焦点已改变，停止测试"
        );
        unsafe {
            SetWindowTextW(edit, w!("前|尾")).unwrap();
            SendMessageW(edit, 0x00b1, Some(WPARAM(1)), Some(LPARAM(2)));
        }
        let before = unsafe { GetClipboardSequenceNumber() };
        let ActionSpec::Local(action) = case.action else {
            unreachable!()
        };
        running.id.set(case.id);
        let invocation = executor.submit(case.id, action, target).unwrap();
        let started = Instant::now();
        loop {
            pump();
            if let Ok(event) = executor.try_recv() {
                assert_eq!((event.id, event.invocation_id), (case.id, invocation));
                if event.outcome.is_terminal() {
                    assert!(
                        matches!(
                            event.outcome,
                            LocalExecutionOutcome::Finished {
                                completed_steps: 1,
                                ..
                            }
                        ),
                        "{event:?}"
                    );
                    break;
                }
            }
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "等待本地文本动作超时"
            );
            thread::sleep(Duration::from_millis(1));
        }
        let expected = format!("前{}尾", case.expected);
        let delivered = Instant::now();
        loop {
            pump();
            if text(edit) == expected || delivered.elapsed() >= Duration::from_secs(1) {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        let observed = text(edit);
        assert_eq!(observed, expected);
        let clipboard_unchanged = before == unsafe { GetClipboardSequenceNumber() };
        assert!(
            clipboard_unchanged,
            "剪贴板序号在测试期间变化，需要重新核验来源"
        );
        evidence.push(json!({"id":case.id,"expected":expected,"observed":observed,"clipboard_sequence_unchanged":clipboard_unchanged}));
    }
    if let Some(path) = std::env::var_os("OMNITRIGGER_LITERAL_TEXT_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&json!({"cases":evidence,"scope":"owned_edit_local_executor_text_only_not_trigger_or_target_application_evidence"})).unwrap()).unwrap();
    }
    eprintln!(
        "本地文本动作实际输入通过：{} 例；未读取或写入剪贴板内容",
        evidence.len()
    );
}
