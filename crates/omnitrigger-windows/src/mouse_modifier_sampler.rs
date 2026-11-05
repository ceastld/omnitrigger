//! 鼠标事件的显式修饰键采样适配；不注入输入，不建立监听或读取用户配置。
//! 多个侧键读取不是原子操作，也不能证明按压来自物理设备。
#[cfg(any(windows, test))]
use omnitrigger_core::{ModifierSides, MouseModifierSnapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseModifierSamplingError {
    pub code: &'static str,
    pub message: &'static str,
}

#[cfg(any(windows, test))]
fn guard_error() -> MouseModifierSamplingError {
    MouseModifierSamplingError {
        code: "modifier_sample_guard_failed",
        message: "鼠标修饰快照的采样条件已失效。",
    }
}

impl std::fmt::Display for MouseModifierSamplingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message)
    }
}
impl std::error::Error for MouseModifierSamplingError {}

/// check由外层核验采样资格与取消；返回后再使用同一单调时钟取得事件评估时间。
/// 注入事件及无效标识在任何侧键读取前拒绝。GetAsyncKeyState的零值不构成来源证明。
#[cfg(windows)]
pub fn sample_mouse_modifiers_if(
    stream_id: u64,
    serial: u64,
    epoch: u64,
    injected: bool,
    check: impl FnMut() -> Result<(), String>,
    sampled_at_ms: impl FnOnce() -> u64,
) -> Result<MouseModifierSnapshot, MouseModifierSamplingError> {
    sample_with(
        stream_id,
        serial,
        epoch,
        injected,
        check,
        |key| {
            Ok(unsafe {
                windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(i32::from(key))
            } < 0)
        },
        sampled_at_ms,
    )
}

#[cfg(any(windows, test))]
fn sample_with(
    stream_id: u64,
    serial: u64,
    epoch: u64,
    injected: bool,
    mut check: impl FnMut() -> Result<(), String>,
    mut key_down: impl FnMut(u16) -> Result<bool, MouseModifierSamplingError>,
    sampled_at_ms: impl FnOnce() -> u64,
) -> Result<MouseModifierSnapshot, MouseModifierSamplingError> {
    if injected {
        return Err(MouseModifierSamplingError {
            code: "modifier_sample_injected",
            message: "注入事件不采集鼠标修饰快照。",
        });
    }
    if stream_id == 0 || serial == 0 || epoch == 0 {
        return Err(MouseModifierSamplingError {
            code: "modifier_sample_identity_invalid",
            message: "鼠标修饰快照缺少有效事件标识。",
        });
    }
    check().map_err(|_| guard_error())?;
    let modifiers = ModifierSides {
        left_shift: key_down(0xa0)?,
        right_shift: key_down(0xa1)?,
        left_control: key_down(0xa2)?,
        right_control: key_down(0xa3)?,
        left_alt: key_down(0xa4)?,
        right_alt: key_down(0xa5)?,
        left_windows: key_down(0x5b)?,
        right_windows: key_down(0x5c)?,
    };
    check().map_err(|_| guard_error())?;
    Ok(MouseModifierSnapshot {
        stream_id,
        event_serial: serial,
        epoch,
        sampled_at_ms: sampled_at_ms(),
        modifiers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[test]
    fn side_reads_are_bounded_ordered_and_time_is_sampled_after_guards_and_reads() {
        let keys = RefCell::new(Vec::new());
        let checked = Cell::new(0);
        let snapshot = sample_with(
            7,
            10,
            2,
            false,
            || {
                checked.set(checked.get() + 1);
                Ok(())
            },
            |key| {
                keys.borrow_mut().push(key);
                Ok(key == 0xa2 || key == 0xa4)
            },
            || {
                assert_eq!(checked.get(), 2);
                assert_eq!(keys.borrow().len(), 8);
                100
            },
        )
        .unwrap();
        assert_eq!(
            *keys.borrow(),
            [0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0x5b, 0x5c]
        );
        assert_eq!(
            (
                snapshot.stream_id,
                snapshot.event_serial,
                snapshot.epoch,
                snapshot.sampled_at_ms
            ),
            (7, 10, 2, 100)
        );
        assert!(snapshot.modifiers.left_control && snapshot.modifiers.left_alt);
        assert!(!snapshot.modifiers.right_control);
    }

    #[test]
    fn injected_and_invalid_requests_never_run_guards_key_reads_or_clock() {
        for (stream, serial, epoch, injected) in [
            (7, 1, 1, true),
            (0, 1, 1, false),
            (7, 0, 1, false),
            (7, 1, 0, false),
        ] {
            assert!(sample_with(
                stream,
                serial,
                epoch,
                injected,
                || panic!("不得检查"),
                |_| panic!("不得采样"),
                || panic!("不得读时钟")
            )
            .is_err());
        }
    }

    #[test]
    fn guard_failures_do_not_classify_strings_and_never_publish_partial_snapshots() {
        for fail_at in [1, 2] {
            let checks = Cell::new(0);
            let reads = Cell::new(0);
            let error = sample_with(
                7,
                1,
                1,
                false,
                || {
                    checks.set(checks.get() + 1);
                    if checks.get() == fail_at {
                        Err("不公开的原因".into())
                    } else {
                        Ok(())
                    }
                },
                |_| {
                    reads.set(reads.get() + 1);
                    Ok(false)
                },
                || panic!("失败不得取得快照时间"),
            )
            .unwrap_err();
            assert_eq!(error.code, "modifier_sample_guard_failed");
            assert_eq!(reads.get(), if fail_at == 1 { 0 } else { 8 });
            assert!(!error.message.contains("不公开"));
        }
    }

    #[test]
    fn backend_failure_stops_at_first_failed_side_without_clock_or_retry() {
        let reads = Cell::new(0);
        let error = sample_with(
            7,
            1,
            1,
            false,
            || Ok(()),
            |_| {
                reads.set(reads.get() + 1);
                if reads.get() == 3 {
                    Err(MouseModifierSamplingError {
                        code: "synthetic_unavailable",
                        message: "合成采样失败。",
                    })
                } else {
                    Ok(false)
                }
            },
            || panic!("失败不得取时钟"),
        )
        .unwrap_err();
        assert_eq!(error.code, "synthetic_unavailable");
        assert_eq!(reads.get(), 3);
    }
}
