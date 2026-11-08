//! 复用既有ForegroundTargets的代次、租约与导出格式；原DesktopHost目标保持不变。
use super::{scoped_clock, ForegroundTargets};
use omnitrigger_actions::{ActionError, CancellationToken};
use omnitrigger_windows::{
    PointingRootHandoffOutcome, ScriptTarget, ScriptTargetSnapshot, WindowGeometryError,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const METHOD: &str = "target.handoff_pointing_root";
type Check<'a> = dyn FnMut() -> Result<(), WindowGeometryError> + 'a;
fn fault(code: &str, message: impl ToString) -> ActionError {
    let mut error = ActionError::new(code, message.to_string());
    error.method = Some(METHOD.into());
    error.effect_state = Some("not_started".into());
    error
}
fn platform_fault(error: WindowGeometryError) -> ActionError {
    fault(error.code, error.message)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedRoot {
    window_handle: String,
    process_id: u32,
    thread_id: u32,
    process_created_file_time: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    expected_root: ExpectedRoot,
    valid_for_ms: u32,
    confirm_timeout_ms: u32,
}
fn parse(params: Value) -> Result<(ScriptTargetSnapshot, Duration, Duration), ActionError> {
    let invalid = || {
        fault(
            "invalid_input",
            "交接需要完整预期根窗身份和明确有效期/确认预算，不接收裸句柄或坐标。",
        )
    };
    if !params.is_object() || !params.get("expected_root").is_some_and(Value::is_object) {
        return Err(invalid());
    }
    let args: Args = serde_json::from_value(params).map_err(|_| invalid())?;
    let handle: usize = args
        .expected_root
        .window_handle
        .parse()
        .map_err(|_| invalid())?;
    let created: u64 = args
        .expected_root
        .process_created_file_time
        .parse()
        .map_err(|_| invalid())?;
    if handle == 0
        || handle > isize::MAX as usize
        || args.expected_root.window_handle != handle.to_string()
        || created == 0
        || args.expected_root.process_created_file_time != created.to_string()
        || args.expected_root.process_id == 0
        || args.expected_root.thread_id == 0
        || !(2..=5000).contains(&args.valid_for_ms)
        || !(1..=1000).contains(&args.confirm_timeout_ms)
        || args.confirm_timeout_ms >= args.valid_for_ms
    {
        return Err(invalid());
    }
    Ok((
        ScriptTargetSnapshot {
            window_handle: handle,
            process_id: args.expected_root.process_id,
            thread_id: args.expected_root.thread_id,
            process_created_file_time: created,
        },
        Duration::from_millis(u64::from(args.valid_for_ms)),
        Duration::from_millis(u64::from(args.confirm_timeout_ms)),
    ))
}

fn execute_with<T>(
    targets: &ForegroundTargets,
    params: Value,
    token: &CancellationToken,
    deadline: Option<Instant>,
    resolve: impl FnOnce() -> Result<T, ActionError>,
    handoff: impl FnOnce(
        &T,
        ScriptTargetSnapshot,
        Duration,
        &mut Check<'_>,
    ) -> Result<PointingRootHandoffOutcome, WindowGeometryError>,
    mut verify: impl FnMut(&T, ScriptTargetSnapshot, &mut Check<'_>) -> Result<(), WindowGeometryError>,
) -> Result<Value, ActionError> {
    let (expected, valid_for, confirm_timeout) = parse(params)?;
    // 与captureForegroundTarget相同：有效捕获尝试先使上次租约失效，失败不借用旧目标。
    let ticket = targets
        .reserve_capture()
        .map_err(|error| fault(&error.code, error.message))?;
    let start = Instant::now();
    let expires = deadline.map_or(start + valid_for, |deadline| {
        deadline.min(start + valid_for)
    });
    let mut attempted = false;
    let mut committed = false;
    let result = (|| {
        let wall = scoped_clock()?;
        let mut guard = || {
            let now = Instant::now();
            let code = if token.is_cancelled() {
                Some("cancelled")
            } else if deadline.is_some_and(|end| now >= end) {
                Some("timeout")
            } else if now >= expires {
                Some("target_expired")
            } else if !targets.capture_current(ticket).map_err(|_| {
                WindowGeometryError::new("target_unavailable", "租约状态不可用。", "not_started")
            })? {
                Some("target_changed")
            } else {
                None
            };
            code.map_or(Ok(()), |code| {
                Err(WindowGeometryError::new(
                    code,
                    "交接条件已失效。",
                    "not_started",
                ))
            })
        };
        guard().map_err(platform_fault)?;
        let original = resolve()?;
        guard().map_err(platform_fault)?;
        let outcome = match handoff(&original, expected, confirm_timeout, &mut guard) {
            Ok(outcome) => {
                attempted = true;
                outcome
            }
            Err(error) => {
                attempted = error.effect_state != "not_started";
                return Err(platform_fault(error));
            }
        };
        guard().map_err(platform_fault)?;
        match outcome {
            PointingRootHandoffOutcome::OsRejected {
                foreground_confirmed,
            } => Ok(json!({
                "requested":true,"requestAccepted":false,"foregroundConfirmed":foreground_confirmed,
                "target":null,"reason":"os_rejected","effectState":"unknown"})),
            PointingRootHandoffOutcome::ConfirmationTimeout => Ok(json!({
                "requested":true,"requestAccepted":true,"foregroundConfirmed":false,
                "target":null,"reason":"confirmation_timeout","effectState":"unknown"})),
            PointingRootHandoffOutcome::Confirmed => {
                verify(&original, expected, &mut guard).map_err(platform_fault)?;
                guard().map_err(platform_fault)?;
                let target = targets.publish_verified(ticket, expected, start, wall, expires)?;
                // 导出/存储后再核验原身份、实际前台、当前指向与代次；失败清除此代租约。
                verify(&original, expected, &mut guard).map_err(platform_fault)?;
                guard().map_err(platform_fault)?;
                committed = true;
                Ok(
                    json!({"requested":true,"requestAccepted":true,"foregroundConfirmed":true,
                    "target":target,"reason":null,"effectState":"unknown"}),
                )
            }
        }
    })();
    let result = if !committed {
        let cleanup = targets.discard_capture(ticket);
        match (result, cleanup) {
            (Ok(_), Err(error)) => Err(error),
            (result, _) => result,
        }
    } else {
        result
    };
    result.map_err(|mut error| {
        error.method = Some(METHOD.into());
        error.effect_state = Some(if attempted { "unknown" } else { "not_started" }.into());
        error
    })
}

pub(super) fn execute(
    targets: &ForegroundTargets,
    original: &Result<ScriptTarget, ActionError>,
    params: Value,
    token: &CancellationToken,
    deadline: Option<Instant>,
) -> Result<Value, ActionError> {
    execute_with(
        targets,
        params,
        token,
        deadline,
        || original.as_ref().map_err(Clone::clone),
        |original, expected, timeout, guard| {
            omnitrigger_windows::handoff_pointing_root_if(original, expected, timeout, guard)
        },
        |original, expected, guard| {
            original.snapshot().map_err(|_| {
                WindowGeometryError::new("target_changed", "原上下文身份失效。", "not_started")
            })?;
            let approved = ScriptTarget::from_snapshot(expected).map_err(|_| {
                WindowGeometryError::new("target_changed", "根窗身份失效。", "not_started")
            })?;
            let observed = omnitrigger_windows::read_pointing_root_if(&approved, &mut *guard)?;
            if observed.root_window != expected {
                return Err(WindowGeometryError::new(
                    "pointing_root_changed",
                    "签发租约前指向根窗已变化。",
                    "not_started",
                ));
            }
            original.snapshot().map_err(|_| {
                WindowGeometryError::new("target_changed", "原上下文身份失效。", "not_started")
            })?;
            approved.ensure_foreground().map_err(|_| {
                WindowGeometryError::new(
                    "target_changed",
                    "签发租约前预期根窗已失焦。",
                    "not_started",
                )
            })?;
            guard()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    fn params() -> Value {
        json!({"expected_root":{"window_handle":"12","process_id":22,"thread_id":32,"process_created_file_time":"42"},"valid_for_ms":5000,"confirm_timeout_ms":50})
    }
    fn seed(targets: &ForegroundTargets) {
        let ticket = targets.reserve_capture().unwrap();
        let now = Instant::now();
        let (snapshot, _, _) = parse(params()).unwrap();
        targets
            .publish_verified(
                ticket,
                snapshot,
                now,
                scoped_clock().unwrap(),
                now + Duration::from_secs(5),
            )
            .unwrap();
    }
    #[test]
    fn malformed_raw_wire_never_reserves_or_resolves_and_cannot_supply_arbitrary_handles() {
        let targets = ForegroundTargets::default();
        seed(&targets);
        for input in [
            json!([]),
            json!({"hwnd":"12"}),
            json!({"expected_root":["12",22,32,"42"],"valid_for_ms":5000,"confirm_timeout_ms":50}),
            {
                let mut p = params();
                p["expected_root"]["window_handle"] = json!("012");
                p
            },
            {
                let mut p = params();
                p["confirm_timeout_ms"] = json!(5000);
                p
            },
        ] {
            let error = execute_with(
                &targets,
                input,
                &CancellationToken::new(),
                None,
                || -> Result<(), ActionError> { panic!("非法参数不解析目标") },
                |_, _, _, _| panic!("非法参数不请求前台"),
                |_, _, _| panic!("非法参数不核验平台"),
            )
            .unwrap_err();
            assert_eq!(error.code, "invalid_input");
            assert_eq!(targets.active.lock().unwrap().0, 1);
            assert!(targets.active.lock().unwrap().1.is_some());
        }
    }
    #[test]
    fn valid_cancel_or_expiry_invalidates_old_lease_before_native_or_resolution() {
        for cancelled in [false, true] {
            let targets = ForegroundTargets::default();
            seed(&targets);
            let token = CancellationToken::new();
            if cancelled {
                token.cancel();
            }
            let error = execute_with(
                &targets,
                params(),
                &token,
                Some(Instant::now()),
                || -> Result<(), ActionError> { panic!("守卫拒绝不解析") },
                |_, _, _, _| panic!("不请求"),
                |_, _, _| panic!("不核验"),
            )
            .unwrap_err();
            assert_eq!(error.code, if cancelled { "cancelled" } else { "timeout" });
            assert_eq!(error.effect_state.as_deref(), Some("not_started"));
            let state = targets.active.lock().unwrap();
            assert_eq!(state.0, 2);
            assert!(state.1.is_none());
        }
    }
    #[test]
    fn only_accepted_and_confirmed_result_reuses_the_existing_exact_identity_lease() {
        let targets = ForegroundTargets::default();
        let calls = Cell::new(0);
        let verifications = Cell::new(0);
        let value = execute_with(
            &targets,
            params(),
            &CancellationToken::new(),
            None,
            || Ok(99),
            |original, expected, _, guard| {
                assert_eq!(*original, 99);
                assert_eq!(expected.window_handle, 12);
                guard()?;
                calls.set(calls.get() + 1);
                Ok(PointingRootHandoffOutcome::Confirmed)
            },
            |original, expected, guard| {
                assert_eq!(*original, 99);
                assert_eq!(expected.process_created_file_time, 42);
                verifications.set(verifications.get() + 1);
                guard()
            },
        )
        .unwrap();
        assert_eq!(calls.get(), 1);
        assert_eq!(verifications.get(), 2);
        assert_eq!(value["target"]["capture_id"], 1);
        assert_eq!(value["target"]["context"]["window_handle"], "12");
        assert_eq!(value["foregroundConfirmed"], true);
        let state = targets.active.lock().unwrap();
        assert_eq!(
            state.1.as_ref().unwrap().context,
            value["target"]["context"]
        );
    }
    #[test]
    fn rejection_and_timeout_never_issue_or_reuse_a_lease_even_if_foreground_is_observed() {
        for outcome in [
            PointingRootHandoffOutcome::OsRejected {
                foreground_confirmed: false,
            },
            PointingRootHandoffOutcome::OsRejected {
                foreground_confirmed: true,
            },
            PointingRootHandoffOutcome::ConfirmationTimeout,
        ] {
            let targets = ForegroundTargets::default();
            seed(&targets);
            let calls = Cell::new(0);
            let value = execute_with(
                &targets,
                params(),
                &CancellationToken::new(),
                None,
                || Ok(()),
                |_, _, _, _| {
                    calls.set(calls.get() + 1);
                    Ok(outcome)
                },
                |_, _, _| panic!("拒绝或超时不签发租约"),
            )
            .unwrap();
            assert_eq!(calls.get(), 1);
            assert_eq!(value["target"], Value::Null);
            assert_eq!(value["effectState"], "unknown");
            assert!(targets.active.lock().unwrap().1.is_none());
        }
    }
    #[test]
    fn late_cancel_or_postpublication_pointer_change_discards_the_new_lease() {
        for cancel in [false, true] {
            let targets = ForegroundTargets::default();
            seed(&targets);
            let token = CancellationToken::new();
            let verifies = Cell::new(0);
            let error = execute_with(
                &targets,
                params(),
                &token,
                None,
                || Ok(()),
                |_, _, _, _| {
                    if cancel {
                        token.cancel();
                    }
                    Ok(PointingRootHandoffOutcome::Confirmed)
                },
                |_, _, guard| {
                    guard()?;
                    verifies.set(verifies.get() + 1);
                    if verifies.get() == 2 {
                        Err(WindowGeometryError::new(
                            "pointing_root_changed",
                            "合成发布后变化",
                            "not_started",
                        ))
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap_err();
            assert_eq!(
                error.code,
                if cancel {
                    "cancelled"
                } else {
                    "pointing_root_changed"
                }
            );
            assert_eq!(error.effect_state.as_deref(), Some("unknown"));
            assert!(targets.active.lock().unwrap().1.is_none());
        }
    }
    #[test]
    fn superseded_handoff_does_not_clear_the_newer_capture_generation() {
        let targets = ForegroundTargets::default();
        let error = execute_with(
            &targets,
            params(),
            &CancellationToken::new(),
            None,
            || Ok(()),
            |_, _, _, _| {
                seed(&targets);
                Ok(PointingRootHandoffOutcome::Confirmed)
            },
            |_, _, _| panic!("已被替代不得签发旧租约"),
        )
        .unwrap_err();
        assert_eq!(error.code, "target_changed");
        assert_eq!(error.effect_state.as_deref(), Some("unknown"));
        let state = targets.active.lock().unwrap();
        assert_eq!(state.0, 2);
        assert!(state.1.is_some());
    }
}
