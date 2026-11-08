//! 只将当前指向且完整身份仍匹配的根窗请求为前台；不改选、不重试、不恢复旧前台。
use crate::{read_pointing_root_if, ScriptTarget, ScriptTargetSnapshot, WindowGeometryError};
use std::time::{Duration, Instant};
use windows::Win32::{Foundation::HWND, UI::WindowsAndMessaging::SetForegroundWindow};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointingRootHandoffOutcome {
    /// 系统返回零；即使同期观察到预期前台，也不据此签发租约。
    OsRejected {
        foreground_confirmed: bool,
    },
    ConfirmationTimeout,
    Confirmed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    BeforeRequest,
    AfterRequest,
}
type Check<'a> = dyn FnMut() -> Result<(), WindowGeometryError> + 'a;

fn fail(code: &'static str, message: &str) -> WindowGeometryError {
    WindowGeometryError::new(code, message, "not_started")
}

// 确定性策略接缝；request只能调用一次，wait仅服务于有界状态观察。
fn perform_with(
    confirmation_end: Instant,
    mut inspect: impl FnMut(Phase, &mut Check<'_>) -> Result<bool, WindowGeometryError>,
    request: impl FnOnce() -> bool,
    mut now: impl FnMut() -> Instant,
    mut wait: impl FnMut(Duration),
    check: &mut Check<'_>,
) -> Result<PointingRootHandoffOutcome, WindowGeometryError> {
    let mut attempted = false;
    let result = (|| {
        check()?;
        if now() >= confirmation_end {
            return Err(fail("timeout", "请求前确认预算已到。"));
        }
        inspect(Phase::BeforeRequest, check)?;
        check()?;
        if now() >= confirmation_end {
            return Err(fail("timeout", "核验后确认预算已到，未请求前台。"));
        }
        attempted = true;
        let accepted = request();
        // 最多101次提交后观察；没有第二次request，也没有改选窗口的回退路径。
        for observation in 0..=100 {
            check()?;
            let foreground = inspect(Phase::AfterRequest, check)?;
            check()?;
            let current = now();
            if !accepted {
                return Ok(PointingRootHandoffOutcome::OsRejected {
                    foreground_confirmed: foreground && current < confirmation_end,
                });
            }
            if current >= confirmation_end {
                return Ok(PointingRootHandoffOutcome::ConfirmationTimeout);
            }
            if foreground {
                return Ok(PointingRootHandoffOutcome::Confirmed);
            }
            if observation == 100 {
                break;
            }
            wait(Duration::from_millis(10).min(confirmation_end.duration_since(current)));
        }
        Ok(PointingRootHandoffOutcome::ConfirmationTimeout)
    })();
    result.map_err(|mut error: WindowGeometryError| {
        if attempted {
            error.effect_state = "unknown";
        }
        error
    })
}

fn inspect(
    original: &ScriptTarget,
    approved: &ScriptTarget,
    expected: ScriptTargetSnapshot,
    phase: Phase,
    check: &mut Check<'_>,
) -> Result<bool, WindowGeometryError> {
    check()?;
    original
        .snapshot()
        .map_err(|_| fail("target_changed", "原上下文身份已失效。"))?;
    let context = if phase == Phase::BeforeRequest {
        original
            .ensure_foreground()
            .map_err(|_| fail("target_changed", "请求前原上下文已失焦。"))?;
        original
    } else if approved.ensure_foreground().is_ok() {
        approved
    } else if original.ensure_foreground().is_ok() {
        original
    } else {
        return Err(fail("target_changed", "前台已变为无关窗口，停止确认。"));
    };
    // 只借用已经批准的原目标/预期根窗作为采样守卫，不重新捕获当前前台。
    let observed = read_pointing_root_if(context, &mut *check)?;
    if observed.root_window != expected {
        return Err(fail(
            "pointing_root_changed",
            "当前指向根窗不再匹配预期身份。",
        ));
    }
    approved
        .snapshot()
        .map_err(|_| fail("target_changed", "预期根窗身份已失效。"))?;
    original
        .snapshot()
        .map_err(|_| fail("target_changed", "原上下文身份已失效。"))?;
    check()?;
    let confirmed = approved.ensure_foreground().is_ok();
    if phase == Phase::BeforeRequest {
        original
            .ensure_foreground()
            .map_err(|_| fail("target_changed", "提交前原上下文已失焦。"))?;
    } else if !confirmed && original.ensure_foreground().is_err() {
        return Err(fail("target_changed", "确认期间前台变为无关窗口。"));
    }
    Ok(confirmed)
}

/// expected仅作为身份比较条件；实际写入目标必须来自本次已守卫的指向根窗查询。
pub fn handoff_pointing_root_if(
    original: &ScriptTarget,
    expected: ScriptTargetSnapshot,
    confirm_timeout: Duration,
    mut check: impl FnMut() -> Result<(), WindowGeometryError>,
) -> Result<PointingRootHandoffOutcome, WindowGeometryError> {
    if !(Duration::from_millis(1)..=Duration::from_millis(1000)).contains(&confirm_timeout) {
        return Err(fail("invalid_input", "前台确认预算须为1–1000ms。"));
    }
    let end = Instant::now() + confirm_timeout;
    check()?;
    let observed = read_pointing_root_if(original, &mut check)?;
    if observed.root_window != expected {
        return Err(fail("target_mismatch", "当前指向根窗不属于预期完整身份。"));
    }
    // 不从调用方句柄直接构造写入目标，只从已匹配的实际观察值捕获身份。
    let approved = ScriptTarget::from_snapshot(observed.root_window)
        .map_err(|_| fail("target_changed", "已观察根窗身份失效。"))?;
    perform_with(
        end,
        |phase, guard| inspect(original, &approved, expected, phase, guard),
        || {
            unsafe {
                SetForegroundWindow(HWND(
                    observed.root_window.window_handle as *mut std::ffi::c_void,
                ))
            }
            .as_bool()
        },
        Instant::now,
        std::thread::sleep,
        &mut check,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn one_request_can_be_accepted_before_foreground_is_confirmed() {
        let start = Instant::now();
        let clock = Cell::new(start);
        let calls = Cell::new(0);
        let after = Cell::new(0);
        let result = perform_with(
            start + Duration::from_millis(50),
            |phase, _| {
                if phase == Phase::BeforeRequest {
                    Ok(false)
                } else {
                    after.set(after.get() + 1);
                    Ok(after.get() == 2)
                }
            },
            || {
                calls.set(calls.get() + 1);
                true
            },
            || clock.get(),
            |duration| clock.set(clock.get() + duration),
            &mut || Ok(()),
        )
        .unwrap();
        assert_eq!(result, PointingRootHandoffOutcome::Confirmed);
        assert_eq!(calls.get(), 1);
        assert_eq!(after.get(), 2);
    }
    #[test]
    fn os_rejection_does_not_retry_even_if_expected_foreground_is_observed_independently() {
        for foreground in [false, true] {
            let start = Instant::now();
            let calls = Cell::new(0);
            let result = perform_with(
                start + Duration::from_millis(50),
                |_, _| Ok(foreground),
                || {
                    calls.set(calls.get() + 1);
                    false
                },
                || start,
                |_| panic!("拒绝后不等待或重试"),
                &mut || Ok(()),
            )
            .unwrap();
            assert_eq!(
                result,
                PointingRootHandoffOutcome::OsRejected {
                    foreground_confirmed: foreground
                }
            );
            assert_eq!(calls.get(), 1);
        }
    }
    #[test]
    fn confirmation_timeout_has_one_request_and_bounded_read_only_observation() {
        let start = Instant::now();
        let clock = Cell::new(start);
        let calls = Cell::new(0);
        let waits = Cell::new(0);
        let result = perform_with(
            start + Duration::from_millis(30),
            |_, _| Ok(false),
            || {
                calls.set(calls.get() + 1);
                true
            },
            || clock.get(),
            |duration| {
                waits.set(waits.get() + 1);
                clock.set(clock.get() + duration);
            },
            &mut || Ok(()),
        )
        .unwrap();
        assert_eq!(result, PointingRootHandoffOutcome::ConfirmationTimeout);
        assert_eq!(calls.get(), 1);
        assert_eq!(waits.get(), 3);
    }
    #[test]
    fn unrelated_pointer_or_foreground_stops_before_or_after_the_only_request() {
        for after_request in [false, true] {
            for code in ["pointing_root_changed", "target_changed"] {
                let start = Instant::now();
                let calls = Cell::new(0);
                let error = perform_with(
                    start + Duration::from_millis(50),
                    |phase, _| {
                        if (phase == Phase::AfterRequest) == after_request {
                            Err(fail(code, "合成无关目标"))
                        } else {
                            Ok(false)
                        }
                    },
                    || {
                        calls.set(calls.get() + 1);
                        true
                    },
                    || start,
                    |_| panic!("失配不继续等待"),
                    &mut || Ok(()),
                )
                .unwrap_err();
                assert_eq!(error.code, code);
                assert_eq!(
                    error.effect_state,
                    if after_request {
                        "unknown"
                    } else {
                        "not_started"
                    }
                );
                assert_eq!(calls.get(), u32::from(after_request));
            }
        }
    }
    #[test]
    fn cancellation_deadline_or_supersession_guard_stops_at_each_boundary() {
        for stop in 1..=4 {
            let start = Instant::now();
            let checks = Cell::new(0);
            let calls = Cell::new(0);
            let error = perform_with(
                start + Duration::from_millis(50),
                |_, _| Ok(true),
                || {
                    calls.set(calls.get() + 1);
                    true
                },
                || start,
                |_| panic!("无需等待"),
                &mut || {
                    checks.set(checks.get() + 1);
                    if checks.get() == stop {
                        Err(fail("cancelled", "合成守卫"))
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap_err();
            assert_eq!(checks.get(), stop);
            assert_eq!(calls.get(), if stop <= 2 { 0 } else { 1 });
            assert_eq!(
                error.effect_state,
                if stop <= 2 { "not_started" } else { "unknown" }
            );
        }
    }
    #[test]
    fn expired_preflight_never_requests_and_a_nonadvancing_clock_still_terminates() {
        let start = Instant::now();
        assert_eq!(
            perform_with(
                start,
                |_, _| panic!("已过期不查询"),
                || panic!("已过期不请求"),
                || start,
                |_| {},
                &mut || Ok(())
            )
            .unwrap_err()
            .effect_state,
            "not_started"
        );
        let calls = Cell::new(0);
        let observations = Cell::new(0);
        let result = perform_with(
            start + Duration::from_millis(50),
            |phase, _| {
                if phase == Phase::AfterRequest {
                    observations.set(observations.get() + 1);
                }
                Ok(false)
            },
            || {
                calls.set(calls.get() + 1);
                true
            },
            || start,
            |_| {},
            &mut || Ok(()),
        )
        .unwrap();
        assert_eq!(result, PointingRootHandoffOutcome::ConfirmationTimeout);
        assert_eq!(calls.get(), 1);
        assert_eq!(observations.get(), 101);
    }
}
