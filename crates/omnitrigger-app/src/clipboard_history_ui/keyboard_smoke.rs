//! 仅向本次创建的 GPUI 窗口投递 Win32 键盘消息，无需系统前台或全局输入。
use super::ClipboardHistoryUi;
use crate::{clipboard_history_send::HistoryLaunchTarget, wheels::tests::Scratch, Workspace};
use gpui::{
    AppContext, Application, AsyncApp, EntityInputHandler, Focusable, KeyDownEvent, Keystroke,
    WindowHandle,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    fs,
    sync::mpsc,
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::{
        PeekMessageW, PostMessageW, MSG, PM_NOREMOVE, WM_CLOSE, WM_KEYDOWN, WM_KEYUP,
    },
};

fn post_key(hwnd: HWND, vk: usize, scan: u32, down: bool) -> Result<(), String> {
    let flags = 1 | (scan << 16) | if down { 0 } else { 3 << 30 };
    let message = if down { WM_KEYDOWN } else { WM_KEYUP };
    unsafe {
        PostMessageW(
            Some(hwnd),
            message,
            WPARAM(vk),
            LPARAM(flags as i32 as isize),
        )
    }
    .map_err(|e| e.to_string())
}

async fn wait_for(
    window: WindowHandle<Workspace>,
    cx: &mut AsyncApp,
    failure: &str,
    check: impl Fn(&ClipboardHistoryUi) -> bool,
) -> Result<(), String> {
    let until = Instant::now() + Duration::from_secs(15);
    loop {
        cx.background_executor()
            .timer(Duration::from_millis(20))
            .await;
        if window
            .update(cx, |v, _, _| check(&v.text_history))
            .map_err(|e| e.to_string())?
        {
            return Ok(());
        }
        if Instant::now() >= until {
            return Err(failure.into());
        }
    }
}

async fn exercise(window: WindowHandle<Workspace>, cx: &mut AsyncApp) -> Result<(), String> {
    wait_for(window, cx, "未加载两条测试摘要", |ui| {
        ui.state.items.len() == 2
    })
    .await?;
    cx.update_window(window.into(), |_, window, app| window.draw(app).clear())
        .map_err(|e| e.to_string())?;
    let hwnd = window
        .update(cx, |v, w, _| {
            if !v.text_history.list_focus.is_focused(w) {
                return Err("首次打开未聚焦历史列表".to_owned());
            }
            match HasWindowHandle::window_handle(&*w)
                .map_err(|e| e.to_string())?
                .as_raw()
            {
                RawWindowHandle::Win32(handle) => Ok(HWND(handle.hwnd.get() as _)),
                _ => Err("预期为 Windows 历史窗口".to_owned()),
            }
        })
        .map_err(|e| e.to_string())??;
    // scan 的第 9 位表示扩展键。End、Home 和 Down 分别验证末项、首项和相邻项。
    for (vk, scan, expected) in [(0x23, 0x14f, "1"), (0x24, 0x147, "2"), (0x28, 0x150, "1")] {
        post_key(hwnd, vk, scan, true)?;
        post_key(hwnd, vk, scan, false)?;
        wait_for(
            window,
            cx,
            &format!("键 {vk:#x} 未选中测试摘要 {expected}"),
            |ui| ui.state.selected.as_deref() == Some(expected),
        )
        .await?;
    }
    post_key(hwnd, 0x0d, 0x1c, true)?;
    wait_for(
        window,
        cx,
        "Enter 按下时未锁定当前项等待释放",
        |ui| ui.enter_send.is_armed() && !ui.sending.pending(),
    )
    .await?;
    post_key(hwnd, 0x0d, 0x1c, false)?;
    wait_for(
        window,
        cx,
        "释放 Enter 后未收到原目标已关闭的回执",
        |ui| {
            ui.state.message == Some("原窗口身份已变化或无法核验，请从原窗口重新打开文本历史。")
                && ui.state.selected.as_deref() == Some("1")
        },
    )
    .await?;

    window
        .update(cx, |v, w, cx| {
            v.text_history.search.read(cx).focus_handle(cx).focus(w);
        })
        .map_err(|e| e.to_string())?;
    cx.update_window(window.into(), |_, w, cx| w.draw(cx).clear())
        .map_err(|e| e.to_string())?;
    for (key, held, composing) in [
        ("ctrl-escape", false, false),
        ("escape", true, false),
        ("escape", false, true),
    ] {
        if composing {
            window
                .update(cx, |v, w, cx| {
                    v.text_history.search.update(cx, |input, cx| {
                        input.replace_and_mark_text_in_range(None, "拼", None, w, cx);
                    });
                })
                .map_err(|e| e.to_string())?;
        }
        let closing = window
            .update(cx, |view, w, cx| {
                view.dismiss_history_on_escape(
                    &KeyDownEvent {
                        keystroke: Keystroke::parse(key).unwrap(),
                        is_held: held,
                    },
                    w,
                    cx,
                );
                // 处理后直接查看本窗消息队列，避免用延时猜测 Esc 是否误发关闭请求。
                unsafe {
                    PeekMessageW(
                        &mut MSG::default(),
                        Some(hwnd),
                        WM_CLOSE,
                        WM_CLOSE,
                        PM_NOREMOVE,
                    )
                    .as_bool()
                }
            })
            .map_err(|e| e.to_string())?;
        if closing {
            return Err(format!(
                "筛选框误收起窗口：{key}，重复={held}，组字={composing}"
            ));
        }
    }
    window
        .update(cx, |v, w, cx| {
            v.text_history
                .search
                .update(cx, |input, cx| input.unmark_text(w, cx));
        })
        .map_err(|e| e.to_string())?;
    post_key(hwnd, 0x1b, 0x01, true)?;
    post_key(hwnd, 0x1b, 0x01, false)?;
    wait_for(
        window,
        cx,
        "筛选框按 Esc 后未进入弹窗关闭流程",
        ClipboardHistoryUi::popup_close_ready,
    )
    .await
}

pub(crate) fn verify(target: HistoryLaunchTarget) {
    let scratch = Scratch::new();
    let config = scratch.bindings();
    let history = config.with_file_name(crate::clipboard_history_store::FILE_NAME);
    let sample = br#"{"version":1,"lastId":"2","revision":"2","items":[{"id":"1","text":"older","createdAt":"1"},{"id":"2","text":"newer","createdAt":"2"}]}"#;
    fs::write(&history, sample).unwrap();
    crate::settings::use_path(config).unwrap();
    let (send, receive) = mpsc::channel();
    Application::new()
        .with_assets(crate::Assets)
        .run(move |cx| {
            crate::message_input::init(cx);
            let window = crate::open_script_history_window(target, true, cx).unwrap();
            cx.spawn(async move |cx| {
                let result = exercise(window, cx).await;
                send.send(result).unwrap();
                cx.update(|cx| cx.quit()).unwrap();
            })
            .detach();
        });
    receive.recv().unwrap().unwrap();
    assert_eq!(fs::read(history).unwrap(), sample);
}

pub(crate) fn verify_profile_search(profile: crate::profiles::Startup) {
    let (send, receive) = mpsc::channel();
    Application::new()
        .with_assets(crate::Assets)
        .run(move |cx| {
            crate::message_input::init(cx);
            let window = cx
                .open_window(Default::default(), |window, cx| {
                    let handle = gpui::Window::window_handle(&*window);
                    cx.new(|cx| {
                        Workspace::new(
                            cx,
                            crate::StartupOptions {
                                mouse_shortcuts: None,
                                wheel_input_policy: omnitrigger_core::WheelInputPolicy::Unfiltered,
                                ui_smoke: false,
                                browser_connector_settings: false,
                                appearance_settings: false,
                                text_history_page: false,
                                text_history_target: None,
                                text_history_ui_window: None,
                                panel: false,
                                wheel: None,
                                wheel_target: None,
                                wheel_editor: false,
                                listen: profile.listen,
                                test_data: profile.test_data,
                                test_profile: profile.test_name,
                            },
                            handle,
                            Err("测试使用系统标题栏".into()),
                        )
                    })
                })
                .unwrap();
            window
                .update(cx, |v, _, cx| {
                    assert!(v.page != crate::Page::TextHistory);
                    v.fire(65, crate::HitSource::Input, cx);
                    assert!(v.page == crate::Page::TextHistory);
                    assert!(!v.executor.is_running(65));
                })
                .unwrap();
            cx.spawn(async move |cx| {
                let result = exercise_profile_search(window, cx).await;
                send.send(result).unwrap();
                cx.update(|cx| cx.quit()).unwrap();
            })
            .detach();
        });
    receive.recv().unwrap().unwrap();
}

async fn exercise_profile_search(
    window: WindowHandle<Workspace>,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    wait_for(window, cx, "没有读到测试档自己的摘要", |ui| {
        ui.state.items.len() == 1 && ui.state.items[0].preview == "test-only"
    })
    .await?;
    window
        .update(cx, |v, w, cx| {
            if !v
                .text_history
                .search
                .read(cx)
                .focus_handle(cx)
                .is_focused(w)
            {
                return Err("搜索未聚焦测试档筛选框".to_string());
            }
            v.text_history
                .search
                .update(cx, |input, cx| input.set_text("test", cx));
            v.text_history.focus_list(w, cx);
            v.test_action(cx);
            Ok(())
        })
        .map_err(|e| e.to_string())??;
    window
        .update(cx, |v, w, cx| {
            let input = v.text_history.search.read(cx);
            if input.text() != "test"
                || !input.focus_handle(cx).is_focused(w)
                || v.executor.is_running(65)
            {
                return Err("再次执行未保留筛选词和焦点，或启动了外部命令".into());
            }
            Ok(())
        })
        .map_err(|e| e.to_string())?
}
