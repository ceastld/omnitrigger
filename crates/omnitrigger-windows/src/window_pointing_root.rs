//! 调用时只读指向窗口及父链根窗身份；观察值不授予输入或切换前台权限。
use crate::{ScriptTarget, ScriptTargetSnapshot, WindowGeometryError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointingRootObservation {
    pub x: i32,
    pub y: i32,
    pub pointing_window: ScriptTargetSnapshot,
    pub root_window: ScriptTargetSnapshot,
}

#[cfg(any(windows, test))]
type Check<'a> = dyn FnMut() -> Result<(), WindowGeometryError> + 'a;
#[cfg(any(windows, test))]
type Pair = (ScriptTargetSnapshot, ScriptTargetSnapshot);

#[cfg(any(windows, test))]
fn observe_with(
    mut point: impl FnMut(&mut Check<'_>) -> Result<(i32, i32), WindowGeometryError>,
    mut identify: impl FnMut((i32, i32), &mut Check<'_>) -> Result<Pair, WindowGeometryError>,
    check: &mut Check<'_>,
) -> Result<PointingRootObservation, WindowGeometryError> {
    check()?;
    let before = point(check)?;
    check()?;
    let first = identify(before, check)?;
    check()?;
    let after = point(check)?;
    check()?;
    let last = identify(after, check)?;
    check()?;
    if before != after || first != last {
        return Err(WindowGeometryError::new(
            "pointing_root_changed",
            "指向坐标、窗口或根窗口身份在查询期间变化。",
            "not_started",
        ));
    }
    Ok(PointingRootObservation {
        x: before.0,
        y: before.1,
        pointing_window: first.0,
        root_window: first.1,
    })
}

#[cfg(windows)]
mod native {
    use super::*;
    use windows::Win32::{
        Foundation::{HWND, POINT},
        UI::WindowsAndMessaging::{GetAncestor, WindowFromPhysicalPoint, GA_ROOT},
    };

    fn handles(point: (i32, i32)) -> Result<(HWND, HWND), WindowGeometryError> {
        let leaf = unsafe {
            WindowFromPhysicalPoint(POINT {
                x: point.0,
                y: point.1,
            })
        };
        let root = unsafe { GetAncestor(leaf, GA_ROOT) };
        if leaf.is_invalid() || root.is_invalid() {
            return Err(WindowGeometryError::new(
                "pointing_root_unavailable",
                "物理坐标处没有可确认的窗口及父链根窗。",
                "not_started",
            ));
        }
        Ok((leaf, root))
    }

    pub(super) fn identify(
        point: (i32, i32),
        check: &mut Check<'_>,
    ) -> Result<Pair, WindowGeometryError> {
        let changed = || {
            WindowGeometryError::new(
                "pointing_root_changed",
                "指向窗口的身份或父链关系已变化。",
                "not_started",
            )
        };
        check()?;
        let (leaf, root) = handles(point)?;
        let capture = |hwnd: HWND| {
            ScriptTarget::capture(crate::ShortcutTarget(hwnd.0 as isize))
                .and_then(|target| target.snapshot())
                .map_err(|_| {
                    WindowGeometryError::new(
                        "pointing_root_unavailable",
                        "无法确认指向窗口的进程/线程及创建时间身份。",
                        "not_started",
                    )
                })
        };
        let leaf_snapshot = capture(leaf)?;
        check()?;
        let root_snapshot = if leaf == root {
            leaf_snapshot
        } else {
            capture(root)?
        };
        check()?;
        if handles(point)? != (leaf, root) {
            return Err(changed());
        }
        // 捕获同一根窗时也复验原身份，不以句柄相等代替PID/TID/进程创建时间。
        ScriptTarget::from_snapshot(leaf_snapshot).map_err(|_| changed())?;
        ScriptTarget::from_snapshot(root_snapshot).map_err(|_| changed())?;
        Ok((leaf_snapshot, root_snapshot))
    }
}

/// 复用本次原目标的身份/前台及现有指针采样会话、输入桌面守卫。
/// 这是调用时观察；不读取监听诊断缓存，不改变原目标或创建显式输入租约。
#[cfg(windows)]
pub fn read_pointing_root_if(
    original: &ScriptTarget,
    mut check: impl FnMut() -> Result<(), WindowGeometryError>,
) -> Result<PointingRootObservation, WindowGeometryError> {
    let mut guard = || {
        check()?;
        original.ensure_foreground().map_err(|_| {
            WindowGeometryError::new(
                "target_changed",
                "本次原目标已失效或不再处于前台。",
                "not_started",
            )
        })
    };
    observe_with(
        |guard| {
            crate::read_window_pointer_state_if(original, guard).map(|state| (state.x, state.y))
        },
        native::identify,
        &mut guard,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn identity(window_handle: usize) -> ScriptTargetSnapshot {
        ScriptTargetSnapshot {
            window_handle,
            process_id: 7,
            thread_id: 8,
            process_created_file_time: 9,
        }
    }

    #[test]
    fn stable_call_time_point_preserves_distinct_child_and_root_identities() {
        let reads = Cell::new(0);
        let observed = observe_with(
            |_| {
                reads.set(reads.get() + 1);
                Ok((-800, 200))
            },
            |point, _| {
                assert_eq!(point, (-800, 200));
                Ok((identity(11), identity(12)))
            },
            &mut || Ok(()),
        )
        .unwrap();
        assert_eq!(reads.get(), 2);
        assert_eq!((observed.x, observed.y), (-800, 200));
        assert_eq!(observed.pointing_window, identity(11));
        assert_eq!(observed.root_window, identity(12));
    }

    #[test]
    fn moved_point_or_changed_child_root_or_process_identity_is_not_published() {
        for changed in ["point", "child", "root", "process", "thread", "created"] {
            let points = Cell::new(0);
            let identities = Cell::new(0);
            let error = observe_with(
                |_| {
                    points.set(points.get() + 1);
                    Ok((
                        if changed == "point" && points.get() == 2 {
                            1
                        } else {
                            0
                        },
                        0,
                    ))
                },
                |_, _| {
                    identities.set(identities.get() + 1);
                    let (mut child, mut root) = (identity(11), identity(12));
                    if identities.get() == 2 {
                        match changed {
                            "child" => child.window_handle += 1,
                            "root" => root.window_handle += 1,
                            "process" => root.process_id += 1,
                            "thread" => root.thread_id += 1,
                            "created" => root.process_created_file_time += 1,
                            _ => {}
                        }
                    }
                    Ok((child, root))
                },
                &mut || Ok(()),
            )
            .unwrap_err();
            assert_eq!(error.code, "pointing_root_changed", "{changed}");
            assert_eq!(error.effect_state, "not_started");
            assert_eq!(points.get(), 2);
            assert_eq!(identities.get(), 2);
        }
    }

    #[test]
    fn every_outer_guard_boundary_rejects_without_publishing_or_retrying() {
        for stop in 1..=5 {
            let checks = Cell::new(0);
            let reads = Cell::new(0);
            let error = observe_with(
                |_| {
                    reads.set(reads.get() + 1);
                    Ok((0, 0))
                },
                |_, _| Ok((identity(11), identity(12))),
                &mut || {
                    checks.set(checks.get() + 1);
                    if checks.get() == stop {
                        Err(WindowGeometryError::new(
                            "cancelled",
                            "合成守卫拒绝",
                            "not_started",
                        ))
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap_err();
            assert_eq!(error.code, "cancelled");
            assert_eq!(checks.get(), stop);
            assert!(reads.get() <= 2);
        }
    }

    #[test]
    fn pointer_or_identity_failure_stops_without_partial_observation() {
        for fail_point in [false, true] {
            let calls = Cell::new(0);
            let error = observe_with(
                |_| {
                    calls.set(calls.get() + 1);
                    if fail_point {
                        Err(WindowGeometryError::new(
                            "pointer_unavailable",
                            "采样失败",
                            "not_started",
                        ))
                    } else {
                        Ok((0, 0))
                    }
                },
                |_, _| {
                    Err(WindowGeometryError::new(
                        "pointing_root_unavailable",
                        "根窗不可用",
                        "not_started",
                    ))
                },
                &mut || Ok(()),
            )
            .unwrap_err();
            assert_eq!(
                error.code,
                if fail_point {
                    "pointer_unavailable"
                } else {
                    "pointing_root_unavailable"
                }
            );
            assert_eq!(calls.get(), 1);
        }
    }
}
