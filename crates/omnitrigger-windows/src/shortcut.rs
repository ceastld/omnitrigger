//! 快捷键动作的有界输入计划；不访问 Hook 的匹配状态。
use std::collections::BTreeSet;

#[cfg(windows)]
pub(crate) const SHORTCUT_TAG: usize = 0x4f4d_534b;
#[cfg(any(windows, test))]
const MODIFIERS: [u16; 8] = [0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0x5b, 0x5c];

fn modifier_family(key: u16) -> Option<u8> {
    match key {
        0xa0 | 0xa1 => Some(0),
        0xa2 | 0xa3 => Some(1),
        0xa4 | 0xa5 => Some(2),
        0x5b | 0x5c => Some(3),
        _ => None,
    }
}

/// 校验一个快捷键：至多四个不同种类的左右修饰键，最后为一个普通键。
/// 使用明确的左右 VK（例如左 Ctrl 为 0xa2），不自动改写配置。
pub fn validate_shortcut(keys: &[u16]) -> Result<(), String> {
    if keys.is_empty() || keys.len() > 5 {
        return Err("发送快捷键需要 1–5 个按键，最后一个必须是普通键。".into());
    }
    let mut families = BTreeSet::new();
    for &key in &keys[..keys.len() - 1] {
        let family = modifier_family(key).ok_or("普通键只能放在快捷键末尾。")?;
        if !families.insert(family) {
            return Err("同一快捷键不能重复包含同类修饰键（包括左右两侧）。".into());
        }
    }
    let terminal = keys[keys.len() - 1];
    if modifier_family(terminal).is_some() || matches!(terminal, 0x10..=0x12) {
        return Err("快捷键末尾必须是普通键，不能仅发送修饰键。".into());
    }
    if !matches!(terminal,
        0x08 | 0x09 | 0x0c | 0x0d | 0x13 | 0x14 | 0x1b |
        0x20..=0x39 | 0x41..=0x5a | 0x5d | 0x5f..=0x87 |
        0x90 | 0x91 | 0xa6..=0xb7 | 0xba..=0xc0 | 0xdb..=0xdf | 0xe2 | 0xf6..=0xfe
    ) {
        return Err(format!("不支持发送虚拟键 VK {terminal:02X}。"));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(any(windows, test))]
struct Stroke {
    key: u16,
    up: bool,
}

#[cfg(any(windows, test))]
fn plan(keys: &[u16], held: &BTreeSet<u16>) -> Result<Vec<Stroke>, String> {
    validate_shortcut(keys)?;
    let terminal = keys[keys.len() - 1];
    if held.contains(&terminal) {
        return Err("目标按键仍被按住，请松开后再发送快捷键。".into());
    }
    if MODIFIERS
        .iter()
        .any(|key| held.contains(key) && !keys.contains(key))
    {
        return Err("存在快捷键以外的修饰键仍被按住，已取消发送。".into());
    }
    // 已按住的目标修饰键只借用，绝不为它生成松开事件。
    let owned: Vec<u16> = keys
        .iter()
        .copied()
        .filter(|key| !held.contains(key))
        .collect();
    Ok(owned
        .iter()
        .map(|&key| Stroke { key, up: false })
        .chain(owned.iter().rev().map(|&key| Stroke { key, up: true }))
        .collect())
}

/// 对部分成功的 SendInput，只收尾本次已经送达且尚未松开的按键。
#[cfg(any(windows, test))]
fn cleanup(plan: &[Stroke], sent: usize) -> Vec<Stroke> {
    let mut held = Vec::new();
    for stroke in plan.iter().take(sent) {
        if stroke.up {
            held.retain(|key| *key != stroke.key);
        } else {
            held.push(stroke.key);
        }
    }
    held.into_iter()
        .rev()
        .map(|key| Stroke { key, up: true })
        .collect()
}

#[cfg(any(windows, test))]
fn extended(key: u16) -> bool {
    matches!(key, 0x21..=0x28 | 0x2c..=0x2e | 0x5b..=0x5d | 0x6f | 0x90 | 0xa3 | 0xa5)
}

/// 向当前前台应用发送一次快捷键；必须在 Hook 与界面线程之外调用。
/// 调用方负责排队并显示错误。本函数不改变焦点，也不松开预先按住的修饰键。
/// Windows 权限隔离或其他 Hook 仍可能拒绝输入；失败收尾不等于无损恢复。
#[cfg(windows)]
pub fn send_shortcut(keys: &[u16]) -> Result<(), String> {
    send_shortcut_to(keys, capture_shortcut_target()?)
}

/// 排队时记录的前台窗口标识，只能由适配层捕获，不暴露原始句柄。
#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShortcutTarget(isize);

#[cfg(windows)]
pub fn capture_shortcut_target() -> Result<ShortcutTarget, String> {
    let foreground = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    if foreground.0.is_null() {
        Err("没有可接收快捷键的前台窗口。".into())
    } else {
        Ok(ShortcutTarget(foreground.0 as isize))
    }
}

/// 向排队时的前台窗口发送快捷键；执行时前台已变化则取消，不主动抢焦点。
/// 必须在 Hook 与界面线程之外调用，其他限制与 `send_shortcut` 相同。
#[cfg(windows)]
pub fn send_shortcut_to(keys: &[u16], target: ShortcutTarget) -> Result<(), String> {
    use std::{mem::size_of, sync::Mutex};
    use windows::Win32::UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
            KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VIRTUAL_KEY,
        },
        WindowsAndMessaging::GetForegroundWindow,
    };

    static SENDING: Mutex<()> = Mutex::new(());
    let _guard = SENDING.lock().map_err(|_| "快捷键发送状态不可用。")?;
    validate_shortcut(keys)?;
    if unsafe { GetForegroundWindow() }.0 as isize != target.0 {
        return Err("前台窗口已切换，已取消发送快捷键。".into());
    }
    let held = MODIFIERS
        .iter()
        .copied()
        .chain(keys.iter().copied())
        .filter(|key| unsafe { GetAsyncKeyState(i32::from(*key)) } < 0)
        .collect();
    let strokes = plan(keys, &held)?;
    fn native(stroke: &Stroke) -> INPUT {
        let mut flags = KEYBD_EVENT_FLAGS(0);
        if stroke.up {
            flags |= KEYEVENTF_KEYUP;
        }
        if extended(stroke.key) {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(stroke.key),
                    dwFlags: flags,
                    dwExtraInfo: SHORTCUT_TAG,
                    ..Default::default()
                },
            },
        }
    }
    let inputs: Vec<INPUT> = strokes.iter().map(native).collect();
    if unsafe { GetForegroundWindow() }.0 as isize != target.0 {
        return Err("前台窗口已切换，已取消发送快捷键。".into());
    }
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) } as usize;
    if sent == inputs.len() {
        return Ok(());
    }
    let releases = cleanup(&strokes, sent);
    let release_inputs: Vec<INPUT> = releases.iter().map(native).collect();
    let mut released = 0;
    // 系统可能只接纳批次的一部分；有限次继续处理剩余释放，绝不重新执行动作。
    for _ in 0..5 {
        if released == release_inputs.len() {
            break;
        }
        released +=
            unsafe { SendInput(&release_inputs[released..], size_of::<INPUT>() as i32) } as usize;
    }
    if released != release_inputs.len() {
        let pending = releases[released..]
            .iter()
            .map(|stroke| crate::format_button(&omnitrigger_core::Button::Key(stroke.key)))
            .collect::<Vec<_>>()
            .join("、");
        return Err(format!(
            "快捷键仅送达 {sent}/{} 个事件，且无法确认 {pending} 已释放；请手动按下并松开这些键。",
            inputs.len()
        ));
    }
    Err(format!(
        "快捷键仅送达 {sent}/{} 个事件；已尝试收尾本次按下，目标应用可能受权限隔离或输入拦截影响。",
        inputs.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_single_editing_keys_and_rejects_ambiguous_combinations() {
        for key in [0x08, 0x25, 0x26, 0x27, 0x28] {
            assert!(validate_shortcut(&[key]).is_ok());
        }
        for keys in [
            vec![],
            vec![0x01],
            vec![0xa2],
            vec![0x11, 0x43],
            vec![0xa2, 0xa3, 0x43],
            vec![0x43, 0xa2],
            vec![0xe7],
        ] {
            assert!(validate_shortcut(&keys).is_err(), "{keys:?}");
        }
    }

    #[test]
    fn borrowed_modifier_is_never_released() {
        let strokes = plan(&[0xa2, 0xa0, 0x43], &BTreeSet::from([0xa2])).unwrap();
        assert_eq!(
            strokes,
            vec![
                Stroke {
                    key: 0xa0,
                    up: false
                },
                Stroke {
                    key: 0x43,
                    up: false
                },
                Stroke {
                    key: 0x43,
                    up: true
                },
                Stroke {
                    key: 0xa0,
                    up: true
                },
            ]
        );
        assert!(plan(&[0xa2, 0x43], &BTreeSet::from([0xa1])).is_err());
        assert!(plan(&[0xa2, 0x43], &BTreeSet::from([0x43])).is_err());
    }

    #[test]
    fn every_partial_prefix_releases_only_owned_unfinished_presses() {
        let strokes = plan(&[0xa2, 0xa0, 0x43], &BTreeSet::from([0xa2])).unwrap();
        let expected: &[&[u16]] = &[&[], &[0xa0], &[0x43, 0xa0], &[0xa0], &[]];
        for (sent, keys) in expected.iter().enumerate() {
            let releases = cleanup(&strokes, sent);
            assert_eq!(
                releases.iter().map(|stroke| stroke.key).collect::<Vec<_>>(),
                *keys
            );
            assert!(releases.iter().all(|stroke| stroke.up));
        }
    }

    #[test]
    fn navigation_and_right_modifiers_use_extended_flags() {
        for key in [0x25, 0x26, 0x27, 0x28, 0x2d, 0x2e, 0xa3, 0xa5, 0x6f] {
            assert!(extended(key));
        }
        for key in [0x08, 0x0d, 0xa0, 0xa2, 0xa4, 0x43] {
            assert!(!extended(key));
        }
    }

    /// 显式运行：cargo test -p omnitrigger-windows interactive_shortcut_smoke -- --ignored --nocapture
    /// 测试会短暂激活专用窗口，结束后尽力恢复原前台。失败或失焦后不继续注入。
    #[cfg(windows)]
    #[test]
    #[ignore = "需要交互式 Windows 桌面，会短暂激活专用测试窗口"]
    fn interactive_shortcut_smoke() {
        use std::{
            thread,
            time::{Duration, Instant},
        };
        use windows::{
            core::w,
            Win32::{
                Foundation::{HWND, LPARAM, WPARAM},
                UI::{
                    Input::KeyboardAndMouse::{GetFocus, SetFocus},
                    WindowsAndMessaging::{
                        CreateWindowExW, DestroyWindow, DispatchMessageW, GetForegroundWindow,
                        GetWindowTextW, PeekMessageW, SendMessageW, SetForegroundWindow,
                        ShowWindow, TranslateMessage, MSG, PM_REMOVE, SW_SHOWNOACTIVATE,
                        WINDOW_EX_STYLE, WINDOW_STYLE, WS_BORDER, WS_CHILD, WS_OVERLAPPEDWINDOW,
                        WS_TABSTOP, WS_VISIBLE,
                    },
                },
            },
        };

        struct TestWindow(HWND, HWND);
        impl Drop for TestWindow {
            fn drop(&mut self) {
                // 仅销毁本测试创建的窗口，随后尽力恢复原前台；不向它注入输入。
                let _ = unsafe { DestroyWindow(self.0) };
                if !self.1 .0.is_null() {
                    let _ = unsafe { SetForegroundWindow(self.1) };
                }
            }
        }
        fn pump() {
            let mut message = MSG::default();
            while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
                let _ = unsafe { TranslateMessage(&message) };
                unsafe { DispatchMessageW(&message) };
            }
        }
        fn selection(edit: HWND) -> (u32, u32) {
            let (mut start, mut end) = (0u32, 0u32);
            unsafe {
                SendMessageW(
                    edit,
                    0x00b0, /* EM_GETSEL */
                    Some(WPARAM((&mut start as *mut u32) as usize)),
                    Some(LPARAM((&mut end as *mut u32) as isize)),
                );
            }
            (start, end)
        }
        fn text(edit: HWND) -> String {
            let mut buffer = [0u16; 32];
            let count = unsafe { GetWindowTextW(edit, &mut buffer) } as usize;
            String::from_utf16(&buffer[..count]).unwrap()
        }

        let previous_foreground = unsafe { GetForegroundWindow() };
        let parent = TestWindow(
            unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("STATIC"),
                    w!("OmniTrigger 快捷键烟测"),
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
            .expect("创建专用测试窗口失败"),
            previous_foreground,
        );
        let edit = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("EDIT"),
                w!("abcd"),
                WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | WINDOW_STYLE(0x0080),
                20,
                35,
                440,
                30,
                Some(parent.0),
                None,
                None,
                None,
            )
        }
        .expect("创建专用测试输入框失败");
        let _ = unsafe { ShowWindow(parent.0, SW_SHOWNOACTIVATE) };
        let _ = unsafe { SetForegroundWindow(parent.0) };
        if unsafe { GetForegroundWindow() } == parent.0 {
            let _ = unsafe { SetFocus(Some(edit)) };
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            pump();
            if unsafe { GetForegroundWindow() } == parent.0 && unsafe { GetFocus() } == edit {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "未取得专用测试窗口的输入焦点，未发送任何快捷键"
            );
            thread::sleep(Duration::from_millis(10));
        }
        let target = capture_shortcut_target().unwrap();
        unsafe {
            SendMessageW(
                edit,
                0x00b1, /* EM_SETSEL */
                Some(WPARAM(4)),
                Some(LPARAM(4)),
            );
        }
        for (keys, expected_text, expected_selection) in [
            (&[0x08][..], "abc", (3, 3)),
            (&[0x25][..], "abc", (2, 2)),
            (&[0xa0, 0x25][..], "abc", (1, 2)),
        ] {
            assert!(
                unsafe { GetForegroundWindow() } == parent.0 && unsafe { GetFocus() } == edit,
                "专用输入框已失焦，未继续发送快捷键"
            );
            send_shortcut_to(keys, target).unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                pump();
                assert!(
                    unsafe { GetForegroundWindow() } == parent.0 && unsafe { GetFocus() } == edit,
                    "发送后专用输入框失焦，已停止后续发送"
                );
                if text(edit) == expected_text && selection(edit) == expected_selection {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "快捷键 {keys:?} 未产生预期结果：文本={:?}，选区={:?}",
                    text(edit),
                    selection(edit)
                );
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}
