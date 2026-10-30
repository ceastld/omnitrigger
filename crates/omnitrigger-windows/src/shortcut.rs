//! 快捷键动作的有界输入计划；不访问 Hook 的匹配状态。
use std::collections::BTreeSet;

#[cfg(windows)]
pub(crate) const SHORTCUT_TAG: usize = 0x4f4d_534b;
#[cfg(any(windows, test))]
pub(crate) const MODIFIERS: [u16; 8] = [0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0x5b, 0x5c];
#[cfg(windows)]
pub(crate) static SENDING: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

/// 单键轻点允许独立修饰键；泛型 Shift/Ctrl/Alt VK 保留其原始含义。
pub fn validate_key_tap(key: u16) -> Result<(), String> {
    if modifier_family(key).is_some() || matches!(key, 0x10..=0x12) {
        Ok(())
    } else {
        validate_shortcut(&[key])
    }
}

/// 一组修饰键跨越 1–16 个依次轻点的普通键，不改变键的顺序或左右侧。
pub fn validate_key_sequence(modifiers: &[u16], keys: &[u16]) -> Result<(), String> {
    if modifiers.len() > 4 {
        return Err("连续按键最多包含四类修饰键。".into());
    }
    if keys.is_empty() || keys.len() > 16 {
        return Err("连续按键需要 1–16 个普通键。".into());
    }
    let mut combination = modifiers.to_vec();
    combination.push(keys[0]);
    validate_shortcut(&combination)?;
    for key in &keys[1..] {
        validate_shortcut(&[*key])?;
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

#[cfg(any(windows, test))]
fn tap_plan(key: u16, held: &BTreeSet<u16>) -> Result<Vec<Stroke>, String> {
    validate_key_tap(key)?;
    if held.contains(&key) || MODIFIERS.iter().any(|modifier| held.contains(modifier)) {
        return Err("轻点目标或修饰键仍被按住，已取消；请先松开这些按键。".into());
    }
    Ok(vec![Stroke { key, up: false }, Stroke { key, up: true }])
}

#[cfg(any(windows, test))]
fn sequence_plan(
    modifiers: &[u16],
    keys: &[u16],
    held: &BTreeSet<u16>,
) -> Result<Vec<Stroke>, String> {
    validate_key_sequence(modifiers, keys)?;
    if keys.iter().any(|key| held.contains(key)) {
        return Err("连续按键中的目标键仍被按住，已取消发送。".into());
    }
    if MODIFIERS
        .iter()
        .any(|key| held.contains(key) && !modifiers.contains(key))
    {
        return Err("存在连续按键以外的修饰键仍被按住，已取消发送。".into());
    }
    let owned: Vec<_> = modifiers
        .iter()
        .copied()
        .filter(|key| !held.contains(key))
        .collect();
    Ok(owned
        .iter()
        .map(|&key| Stroke { key, up: false })
        .chain(
            keys.iter()
                .flat_map(|&key| [Stroke { key, up: false }, Stroke { key, up: true }]),
        )
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
    matches!(key, 0x21..=0x28 | 0x2c..=0x2e | 0x5b..=0x5d | 0x6f | 0x90 | 0xa3 | 0xa5 | 0xad..=0xaf)
}

#[cfg(any(windows, test))]
fn bare_volume_command(keys: &[u16]) -> Option<u16> {
    match keys {
        [0xad] => Some(8),  // APPCOMMAND_VOLUME_MUTE
        [0xae] => Some(9),  // APPCOMMAND_VOLUME_DOWN
        [0xaf] => Some(10), // APPCOMMAND_VOLUME_UP
        _ => None,
    }
}

/// 向当前前台应用发送一次快捷键；必须在 Hook 与界面线程之外调用。
/// 调用方负责排队并显示错误。本函数不改变焦点，也不松开预先按住的修饰键。
/// 裸音量键改为向 Windows Shell 提交系统媒体命令，避免被前台应用吞掉；不注入替代按键。
/// Windows 权限隔离或其他 Hook 仍可能拒绝输入；失败收尾不等于无损恢复。
#[cfg(windows)]
pub fn send_shortcut(keys: &[u16]) -> Result<(), String> {
    send_shortcut_to(keys, capture_shortcut_target()?)
}

/// 排队时记录的前台窗口标识，只能由适配层捕获，不暴露原始句柄。
#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShortcutTarget(pub(crate) isize);

#[cfg(windows)]
pub fn capture_shortcut_target() -> Result<ShortcutTarget, String> {
    let foreground = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    if foreground.0.is_null() {
        Err("没有可接收快捷键的前台窗口。".into())
    } else {
        Ok(ShortcutTarget(foreground.0 as isize))
    }
}

/// 确认原目标仍有效且仍为前台；只读核验，不改变窗口焦点。
/// 多步骤动作应在每个需要原窗口的步骤之前重新核验。
#[cfg(windows)]
pub fn ensure_shortcut_target(target: ShortcutTarget) -> Result<(), String> {
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{GetForegroundWindow, IsWindow},
    };
    let hwnd = HWND(target.0 as *mut std::ffi::c_void);
    if hwnd.0.is_null() || !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return Err("原目标窗口已关闭或失效，已取消动作。".into());
    }
    if unsafe { GetForegroundWindow() } != hwnd {
        return Err("前台窗口已切换，已取消动作。".into());
    }
    Ok(())
}

/// 尝试交还启动前捕获的前台窗口，不注入按键，也不绕过系统前台权限限制。
/// 调用方应先确认当前前台仍是自己的窗口，避免覆盖用户已经完成的窗口切换。
/// 目标已失效时返回错误；系统拒绝前台切换时返回 `Ok(false)`。
#[cfg(windows)]
pub fn restore_shortcut_target(target: ShortcutTarget) -> Result<bool, String> {
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{GetForegroundWindow, IsWindow, SetForegroundWindow},
    };
    let hwnd = HWND(target.0 as *mut std::ffi::c_void);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return Err("原目标窗口已关闭或失效，无法恢复前台。".into());
    }
    if unsafe { GetForegroundWindow() } == hwnd {
        return Ok(true);
    }
    let _ = unsafe { SetForegroundWindow(hwnd) };
    Ok(unsafe { GetForegroundWindow() } == hwnd)
}

/// 向排队时的前台窗口发送快捷键；执行时前台已变化则取消，不主动抢焦点。
/// 必须在 Hook 与界面线程之外调用，其他限制与 `send_shortcut` 相同。
#[cfg(windows)]
pub fn send_shortcut_to(keys: &[u16], target: ShortcutTarget) -> Result<(), String> {
    let _guard = SENDING.lock().map_err(|_| "快捷键发送状态不可用。")?;
    validate_shortcut(keys)?;
    ensure_shortcut_target(target)?;
    let strokes = plan(keys, &held_keys(keys))?;
    if let Some(command) = bare_volume_command(keys) {
        return send_volume_command(command, target);
    }
    send_strokes(&strokes, target, "快捷键")
}

/// 轻点一个键，包括独立 Alt/Ctrl/Shift/Win；不能轻点已经按住的键。
/// 必须由工作线程调用，不更改焦点，不释放用户预先按住的按键。
#[cfg(windows)]
pub fn send_key_tap_to(key: u16, target: ShortcutTarget) -> Result<(), String> {
    let _guard = SENDING.lock().map_err(|_| "按键发送状态不可用。")?;
    validate_key_tap(key)?;
    ensure_shortcut_target(target)?;
    let strokes = tap_plan(key, &held_keys(&[key]))?;
    if let Some(command) = bare_volume_command(&[key]) {
        return send_volume_command(command, target);
    }
    send_strokes(&strokes, target, "单键轻点")
}

/// 调用方持有 SENDING，且已完成原前台和物理按键状态校验。只选择一次路由，不进行注入回退。
#[cfg(windows)]
fn send_volume_command(command: u16, target: ShortcutTarget) -> Result<(), String> {
    use windows::Win32::{
        Foundation::{GetLastError, SetLastError, WIN32_ERROR},
        UI::WindowsAndMessaging::{GetShellWindow, SendMessageTimeoutW},
    };
    ensure_shortcut_target(target)?;
    let shell = unsafe { GetShellWindow() };
    send_volume_message_with(
        command,
        target,
        shell,
        |shell, message, source, packed, flags, timeout| {
            let mut response = 0;
            unsafe { SetLastError(WIN32_ERROR(0)) };
            let delivered = unsafe {
                SendMessageTimeoutW(
                    shell,
                    message,
                    source,
                    packed,
                    flags,
                    timeout,
                    Some(&mut response),
                )
            };
            if delivered.0 == 0 {
                let error = unsafe { GetLastError() };
                return Err(format!(
                    "无法确认 Windows Shell 音量命令已送达或完成（错误 {}）；结果未知，不自动重试。",
                    error.0
                ));
            }
            Ok(response)
        },
    )
}

#[cfg(windows)]
fn send_volume_message_with(
    command: u16,
    source: ShortcutTarget,
    shell: windows::Win32::Foundation::HWND,
    send: impl FnOnce(
        windows::Win32::Foundation::HWND,
        u32,
        windows::Win32::Foundation::WPARAM,
        windows::Win32::Foundation::LPARAM,
        windows::Win32::UI::WindowsAndMessaging::SEND_MESSAGE_TIMEOUT_FLAGS,
        u32,
    ) -> Result<usize, String>,
) -> Result<(), String> {
    use windows::Win32::{
        Foundation::{LPARAM, WPARAM},
        UI::WindowsAndMessaging::{SMTO_ABORTIFHUNG, SMTO_ERRORONEXIT, WM_APPCOMMAND},
    };
    if shell.is_invalid() {
        return Err("Windows Shell 不可用，无法发送系统音量命令；未注入替代按键。".into());
    }
    // 高字为命令，输入来源 FAPPCOMMAND_KEY 和修饰键状态均为零；wParam 保留触发前台身份。
    // 这里只确认消息调用已完成。Explorer 可在执行媒体命令后返回零，不能据此判定失败；
    // 接收方返回值也不能作为硬件音量读回或系统浮层显示成功的证明。
    let _response = send(
        shell,
        WM_APPCOMMAND,
        WPARAM(source.0 as usize),
        LPARAM((command as isize) << 16),
        SMTO_ABORTIFHUNG | SMTO_ERRORONEXIT,
        1000,
    )?;
    Ok(())
}

/// 保持一组修饰键，依次轻点多个普通键。已按住的目标修饰键只借用，不松开。
/// 必须由工作线程调用；与普通快捷键和 Unicode 文本发送共用串行门闩。
#[cfg(windows)]
pub fn send_key_sequence_to(
    modifiers: &[u16],
    keys: &[u16],
    target: ShortcutTarget,
) -> Result<(), String> {
    let _guard = SENDING.lock().map_err(|_| "连续按键发送状态不可用。")?;
    validate_key_sequence(modifiers, keys)?;
    ensure_shortcut_target(target)?;
    let strokes = sequence_plan(modifiers, keys, &held_keys(keys))?;
    send_strokes(&strokes, target, "连续按键")
}

#[cfg(windows)]
fn held_keys(keys: &[u16]) -> BTreeSet<u16> {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    MODIFIERS
        .iter()
        .copied()
        .chain(keys.iter().copied())
        .filter(|key| unsafe { GetAsyncKeyState(i32::from(*key)) } < 0)
        .collect()
}

#[cfg(windows)]
fn native_input(stroke: &Stroke) -> windows::Win32::UI::Input::KeyboardAndMouse::INPUT {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY,
        KEYEVENTF_KEYUP, VIRTUAL_KEY,
    };
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

/// 调用方必须持有 SENDING；清理只涉及当前计划已提交的自有按下。
#[cfg(windows)]
fn send_strokes(strokes: &[Stroke], target: ShortcutTarget, label: &str) -> Result<(), String> {
    use std::mem::size_of;
    use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT};
    let inputs: Vec<INPUT> = strokes.iter().map(native_input).collect();
    ensure_shortcut_target(target)?;
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) } as usize;
    if sent == inputs.len() {
        return Ok(());
    }
    let releases = cleanup(strokes, sent);
    let release_inputs: Vec<INPUT> = releases.iter().map(native_input).collect();
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
            "{label}仅送达 {sent}/{} 个事件，且无法确认 {pending} 已释放；请手动按下并松开这些键。",
            inputs.len()
        ));
    }
    Err(format!(
        "{label}仅送达 {sent}/{} 个事件；已尝试收尾本次按下，目标应用可能受权限隔离或输入拦截影响。",
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

    #[test]
    fn only_bare_volume_keys_select_system_media_commands() {
        for (key, command) in [(0xad, 8), (0xae, 9), (0xaf, 10)] {
            assert_eq!(bare_volume_command(&[key]), Some(command));
            for modifier in MODIFIERS {
                assert_eq!(bare_volume_command(&[modifier, key]), None);
                assert!(plan(&[modifier, key], &BTreeSet::new()).is_ok());
            }
            assert!(plan(&[key], &BTreeSet::from([key])).is_err());
            assert!(tap_plan(key, &BTreeSet::from([key])).is_err());
            for modifier in MODIFIERS {
                assert!(plan(&[key], &BTreeSet::from([modifier])).is_err());
                assert!(tap_plan(key, &BTreeSet::from([modifier])).is_err());
            }
        }
        for keys in [vec![], vec![0x41], vec![0xb0], vec![0xae, 0xaf]] {
            assert_eq!(bare_volume_command(&keys), None);
        }
    }

    #[cfg(windows)]
    #[test]
    fn system_media_message_preserves_source_and_key_origin_and_uses_one_bounded_submission() {
        use std::cell::Cell;
        use windows::Win32::{
            Foundation::HWND,
            UI::WindowsAndMessaging::{SMTO_ABORTIFHUNG, SMTO_ERRORONEXIT, WM_APPCOMMAND},
        };
        let shell = HWND(0x5678usize as *mut std::ffi::c_void);
        let source = ShortcutTarget(0x1234);
        for command in [8, 9, 10] {
            let calls = Cell::new(0);
            send_volume_message_with(
                command,
                source,
                shell,
                |target, message, origin, packed, flags, timeout| {
                    calls.set(calls.get() + 1);
                    assert_eq!(target, shell);
                    assert_eq!(message, WM_APPCOMMAND);
                    assert_eq!(origin.0, 0x1234);
                    assert_eq!(packed.0, (command as isize) << 16);
                    assert_eq!(flags, SMTO_ABORTIFHUNG | SMTO_ERRORONEXIT);
                    assert_eq!(timeout, 1000);
                    Ok(1)
                },
            )
            .unwrap();
            assert_eq!(calls.get(), 1);
        }
        assert!(
            send_volume_message_with(10, source, HWND::default(), |_, _, _, _, _, _| {
                panic!("缺少 Shell 时不能提交消息")
            })
            .unwrap_err()
            .contains("Shell 不可用")
        );
        let calls = Cell::new(0);
        send_volume_message_with(10, source, shell, |_, _, _, _, _, _| {
            calls.set(calls.get() + 1);
            Ok(0)
        })
        .unwrap();
        assert_eq!(calls.get(), 1);
        assert_eq!(
            send_volume_message_with(10, source, shell, |_, _, _, _, _, _| {
                calls.set(calls.get() + 1);
                Err("发送超时".into())
            })
            .unwrap_err(),
            "发送超时"
        );
        assert_eq!(calls.get(), 2);
    }

    #[cfg(windows)]
    #[test]
    fn volume_key_inputs_and_partial_cleanup_keep_extended_flags_and_own_injection_tag() {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            INPUT_KEYBOARD, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
        };
        for key in [0xad, 0xae, 0xaf] {
            let strokes = plan(&[key], &BTreeSet::new()).unwrap();
            assert_eq!(
                strokes,
                [Stroke { key, up: false }, Stroke { key, up: true }]
            );
            for (stroke, expected_flags) in strokes.iter().zip([
                KEYEVENTF_EXTENDEDKEY,
                KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP,
            ]) {
                let input = native_input(stroke);
                assert_eq!(input.r#type, INPUT_KEYBOARD);
                let keyboard = unsafe { input.Anonymous.ki };
                assert_eq!(keyboard.wVk.0, key);
                assert_eq!(keyboard.dwFlags, expected_flags);
                assert_eq!(keyboard.dwExtraInfo, SHORTCUT_TAG);
            }
            assert!(cleanup(&strokes, 0).is_empty());
            let releases = cleanup(&strokes, 1);
            assert_eq!(releases, [Stroke { key, up: true }]);
            let keyboard = unsafe { native_input(&releases[0]).Anonymous.ki };
            assert_eq!(keyboard.wVk.0, key);
            assert_eq!(keyboard.dwFlags, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP);
            assert_eq!(keyboard.dwExtraInfo, SHORTCUT_TAG);
            assert!(cleanup(&strokes, 2).is_empty());
        }
        // 不扩大其他浏览器或媒体键的既有扩展语义。
        for key in [0xa6, 0xac, 0xb0, 0xb7] {
            assert!(!extended(key));
        }
    }

    #[test]
    fn modifier_tap_requires_every_physical_modifier_released() {
        for key in MODIFIERS.into_iter().chain([0x10, 0x11, 0x12, 0x43]) {
            assert!(validate_key_tap(key).is_ok());
            let strokes = tap_plan(key, &BTreeSet::new()).unwrap();
            assert_eq!(
                strokes,
                [Stroke { key, up: false }, Stroke { key, up: true }]
            );
            assert_eq!(cleanup(&strokes, 1), [Stroke { key, up: true }]);
            assert!(cleanup(&strokes, 2).is_empty());
            assert!(tap_plan(key, &BTreeSet::from([key])).is_err());
            for held in MODIFIERS {
                assert!(tap_plan(key, &BTreeSet::from([held])).is_err());
            }
        }
        assert!(validate_key_tap(0xe7).is_err());
    }

    #[test]
    fn sequence_keeps_modifier_owned_once_and_preserves_terminal_order() {
        let strokes =
            sequence_plan(&[0xa2, 0xa0], &[0x4b, 0x43, 0x43], &BTreeSet::from([0xa2])).unwrap();
        assert_eq!(
            strokes,
            [
                Stroke {
                    key: 0xa0,
                    up: false
                },
                Stroke {
                    key: 0x4b,
                    up: false
                },
                Stroke {
                    key: 0x4b,
                    up: true
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
        for sent in 0..=strokes.len() {
            let releases = cleanup(&strokes, sent);
            assert!(releases
                .iter()
                .all(|stroke| stroke.up && stroke.key != 0xa2));
            let mut held = BTreeSet::new();
            for stroke in strokes.iter().take(sent).chain(releases.iter()) {
                if stroke.up {
                    held.remove(&stroke.key);
                } else {
                    held.insert(stroke.key);
                }
            }
            assert!(held.is_empty(), "部分提交 {sent} 未收尾");
        }
        assert!(sequence_plan(&[0xa2], &[0x43], &BTreeSet::from([0xa3])).is_err());
        assert!(sequence_plan(&[0xa2], &[0x43], &BTreeSet::from([0x43])).is_err());
    }

    #[test]
    fn sequence_rejects_ambiguous_modifiers_and_bounds_terminal_count() {
        assert!(validate_key_sequence(&[], &[0x41; 16]).is_ok());
        for (modifiers, keys) in [
            (vec![], vec![]),
            (vec![], vec![0x41; 17]),
            (vec![0xa2, 0xa3], vec![0x43]),
            (vec![0x11], vec![0x43]),
            (vec![0xa2], vec![0x43, 0xa0]),
            (vec![0x43], vec![0x41]),
        ] {
            assert!(validate_key_sequence(&modifiers, &keys).is_err());
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
