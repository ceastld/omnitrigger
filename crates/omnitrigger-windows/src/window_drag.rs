//! 同一动作worker中的有界窗口调整适配；不创建线程/Hook/输入引擎，不注入或消费按钮。
use crate::{WindowGeometryError, WindowRect};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowDragMode {
    Move,
    Resize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowDragButton {
    Left,
    Right,
    Middle,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowSnapOptions {
    pub bounds: WindowRect,
    pub distance: u32,
    pub wait_ms: u32,
    pub initial_elapsed_ms: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowDragOptions {
    pub mode: WindowDragMode,
    pub button: WindowDragButton,
    pub max_duration_ms: u32,
    pub max_updates: u32,
    pub snap: Option<WindowSnapOptions>,
}
impl WindowDragOptions {
    pub fn validate(self) -> Result<Self, WindowGeometryError> {
        if !(20..=2000).contains(&self.max_duration_ms) || !(1..=64).contains(&self.max_updates) {
            return Err(invalid("持续预算需要20–2000ms，提交预算需要1–64。"));
        }
        if let Some(snap) = self.snap {
            snap.bounds.validate()?;
            if self.mode != WindowDragMode::Move
                || snap.distance > 100
                || snap.wait_ms > 1000
                || snap.initial_elapsed_ms > 60000
            {
                return Err(invalid(
                    "吸附只用于移动；距离0–100、等待0–1000ms、明确初始间隔0–60000ms。",
                ));
            }
        }
        Ok(self)
    }
}
fn invalid(message: &str) -> WindowGeometryError {
    WindowGeometryError::new("invalid_input", message, "not_started")
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowDragStop {
    ButtonNotDownOrUnavailable,
    DurationLimit,
    UpdateLimit,
}
impl WindowDragStop {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ButtonNotDownOrUnavailable => "button_not_down_or_unavailable",
            Self::DurationLimit => "duration_limit",
            Self::UpdateLimit => "update_limit",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowDragStopped {
    pub reason: WindowDragStop,
    pub accepted_updates: u32,
    pub poll_samples: u32,
    pub effect_state: &'static str,
}

#[cfg(any(windows, test))]
fn down(state: crate::WindowPointerState, button: WindowDragButton) -> bool {
    let state = match button {
        WindowDragButton::Left => state.left,
        WindowDragButton::Right => state.right,
        WindowDragButton::Middle => state.middle,
    };
    state == crate::PointerButtonState::Down
}
#[cfg(any(windows, test))]
fn coordinate(value: i64) -> Result<i32, WindowGeometryError> {
    i32::try_from(value).map_err(|_| invalid("光标增量使坐标溢出，未裁剪或回绕。"))
}
#[cfg(any(windows, test))]
fn moved(
    rect: WindowRect,
    dx: i64,
    dy: i64,
    mode: WindowDragMode,
) -> Result<WindowRect, WindowGeometryError> {
    let result = match mode {
        WindowDragMode::Move => WindowRect {
            left: coordinate(i64::from(rect.left) + dx)?,
            top: coordinate(i64::from(rect.top) + dy)?,
            ..rect
        },
        WindowDragMode::Resize => WindowRect {
            width: coordinate(i64::from(rect.width) + dx)?,
            height: coordinate(i64::from(rect.height) + dy)?,
            ..rect
        },
    };
    result.validate()
}
#[cfg(any(windows, test))]
fn snap_step(
    rect: WindowRect,
    options: WindowSnapOptions,
    side: u8,
    now: i64,
    last: &mut i64,
) -> Result<Option<WindowRect>, WindowGeometryError> {
    let right = i64::from(options.bounds.left) + i64::from(options.bounds.width);
    let top = i64::from(options.bounds.top);
    let left = i64::from(options.bounds.left);
    let distance = i64::from(options.distance);
    let near = match side {
        0 => i64::from(rect.left) > left && i64::from(rect.left) < left + distance,
        1 => i64::from(rect.top) > top && i64::from(rect.top) < top + distance,
        _ => {
            i64::from(rect.left) + i64::from(rect.width) > right - distance
                && i64::from(rect.left) + i64::from(rect.width) < right
        }
    };
    if !near {
        return Ok(None);
    }
    let result = if now - *last > i64::from(options.wait_ms) {
        Some(
            match side {
                0 => WindowRect {
                    left: options.bounds.left,
                    ..rect
                },
                1 => WindowRect {
                    top: options.bounds.top,
                    ..rect
                },
                _ => WindowRect {
                    left: coordinate(right - i64::from(rect.width))?,
                    ..rect
                },
            }
            .validate()?,
        )
    } else {
        None
    };
    *last = now;
    Ok(result)
}
#[cfg(test)]
fn snaps(
    rect: WindowRect,
    options: WindowSnapOptions,
    now: i64,
    last: &mut i64,
) -> Result<Vec<WindowRect>, WindowGeometryError> {
    let mut result = Vec::new();
    for side in 0..3 {
        if let Some(next) = snap_step(rect, options, side, now, last)? {
            result.push(next);
        }
    }
    Ok(result)
}

#[cfg(any(windows, test))]
trait Backend {
    fn check(&mut self) -> Result<(), WindowGeometryError>;
    fn sample(&mut self) -> Result<crate::WindowPointerState, WindowGeometryError>;
    fn geometry(&mut self) -> Result<crate::WindowGeometry, WindowGeometryError>;
    fn set(&mut self, rect: WindowRect) -> Result<(), WindowGeometryError>;
    fn wait(&mut self);
    fn now_ms(&self) -> u64;
}
#[cfg(any(windows, test))]
fn run(
    options: WindowDragOptions,
    backend: &mut impl Backend,
) -> Result<WindowDragStopped, WindowGeometryError> {
    let options = options.validate()?;
    let mut accepted = 0;
    let mut samples = 0;
    let mut attempted = false;
    let result = (|| {
        backend.check()?;
        let start = backend.now_ms();
        let first = backend.sample()?;
        samples += 1;
        if !down(first, options.button) {
            return Ok(WindowDragStop::ButtonNotDownOrUnavailable);
        }
        let mut previous = first;
        let mut last_snap = options
            .snap
            .map_or(0, |snap| -i64::from(snap.initial_elapsed_ms));
        loop {
            backend.check()?;
            if accepted >= options.max_updates {
                return Ok(WindowDragStop::UpdateLimit);
            }
            if samples >= 201 {
                return Ok(WindowDragStop::DurationLimit);
            }
            if backend.now_ms().saturating_sub(start) >= u64::from(options.max_duration_ms) {
                return Ok(WindowDragStop::DurationLimit);
            }
            backend.wait();
            backend.check()?;
            let elapsed = backend.now_ms().saturating_sub(start);
            if elapsed >= u64::from(options.max_duration_ms) {
                return Ok(WindowDragStop::DurationLimit);
            }
            let geometry = backend.geometry()?;
            let current = backend.sample()?;
            samples += 1;
            if !down(current, options.button) {
                return Ok(WindowDragStop::ButtonNotDownOrUnavailable);
            }
            if geometry.maximized {
                return Err(WindowGeometryError::new(
                    "window_not_normal",
                    "有界调整不隐式还原最大化窗口。",
                    "not_started",
                ));
            }
            let dx = i64::from(current.x) - i64::from(previous.x);
            let dy = i64::from(current.y) - i64::from(previous.y);
            if options.mode == WindowDragMode::Resize && dx.abs() < 10 && dy.abs() < 10 {
                continue;
            }
            let request = moved(geometry.visible_rect, dx, dy, options.mode)?;
            backend.check()?;
            match backend.set(request) {
                Ok(()) => {
                    accepted += 1;
                    attempted = true;
                }
                Err(error) => {
                    attempted |= error.effect_state == "unknown";
                    if error.code == "drag_button_not_down" {
                        return Ok(WindowDragStop::ButtonNotDownOrUnavailable);
                    }
                    return Err(error);
                }
            }
            previous = current;
            if let Some(snap) = options.snap {
                // 每一活动边分别取时，仍用同一原计划矩形；同一调用共享last_snap。
                for side in 0..3 {
                    backend.check()?;
                    let now = backend.now_ms().saturating_sub(start);
                    if now >= u64::from(options.max_duration_ms) {
                        return Ok(WindowDragStop::DurationLimit);
                    }
                    if let Some(next) = snap_step(request, snap, side, now as i64, &mut last_snap)?
                    {
                        if accepted >= options.max_updates {
                            return Ok(WindowDragStop::UpdateLimit);
                        }
                        match backend.set(next) {
                            Ok(()) => {
                                accepted += 1;
                                attempted = true;
                            }
                            Err(error) => {
                                attempted |= error.effect_state == "unknown";
                                if error.code == "drag_button_not_down" {
                                    return Ok(WindowDragStop::ButtonNotDownOrUnavailable);
                                }
                                return Err(error);
                            }
                        }
                    }
                }
            }
        }
    })();
    match result {
        Ok(reason) => Ok(WindowDragStopped {
            reason,
            accepted_updates: accepted,
            poll_samples: samples,
            effect_state: if attempted { "unknown" } else { "not_started" },
        }),
        Err(error) if error.code == "window_drag_budget_elapsed" => Ok(WindowDragStopped {
            reason: WindowDragStop::DurationLimit,
            accepted_updates: accepted,
            poll_samples: samples,
            effect_state: if attempted || error.effect_state == "unknown" {
                "unknown"
            } else {
                "not_started"
            },
        }),
        Err(mut error) => {
            if attempted {
                error.effect_state = "unknown";
            }
            Err(error)
        }
    }
}

#[cfg(windows)]
pub fn track_window_drag_if(
    target: &crate::ScriptTarget,
    options: WindowDragOptions,
    mut check: impl FnMut() -> Result<(), WindowGeometryError>,
) -> Result<WindowDragStopped, WindowGeometryError> {
    use std::{
        sync::Mutex,
        time::{Duration, Instant},
    };
    static ACTIVE: Mutex<()> = Mutex::new(());
    options.validate()?;
    let _active = ACTIVE.try_lock().map_err(|_| {
        WindowGeometryError::new(
            "window_drag_busy",
            "有界窗口调整互斥门当前不可用，未排队或停止旧调用。",
            "not_started",
        )
    })?;
    struct Native<'a, F> {
        target: &'a crate::ScriptTarget,
        check: F,
        start: Instant,
        limit: Instant,
        button: WindowDragButton,
    }
    impl<F: FnMut() -> Result<(), WindowGeometryError>> Native<'_, F> {
        fn guard(&mut self) -> Result<(), WindowGeometryError> {
            (self.check)()?;
            if Instant::now() >= self.limit {
                return Err(WindowGeometryError::new(
                    "window_drag_budget_elapsed",
                    "有界调整预算已到。",
                    "not_started",
                ));
            }
            Ok(())
        }
    }
    impl<F: FnMut() -> Result<(), WindowGeometryError>> Backend for Native<'_, F> {
        fn check(&mut self) -> Result<(), WindowGeometryError> {
            self.guard()?;
            crate::window_pointer::ensure_context(self.target)?;
            self.guard()
        }
        fn sample(&mut self) -> Result<crate::WindowPointerState, WindowGeometryError> {
            let target = self.target;
            crate::read_window_pointer_state_if(target, || self.guard())
        }
        fn geometry(&mut self) -> Result<crate::WindowGeometry, WindowGeometryError> {
            let target = self.target;
            target.window_geometry_if(|| self.check())
        }
        fn set(&mut self, rect: WindowRect) -> Result<(), WindowGeometryError> {
            let state = self.sample()?;
            if !down(state, self.button) {
                return Err(WindowGeometryError::new(
                    "drag_button_not_down",
                    "按钮已非明确按下或采样不可用。",
                    "not_started",
                ));
            }
            let target = self.target;
            target.set_window_rect_if(rect, crate::WindowRectFrame::Visible, || self.check())
        }
        fn wait(&mut self) {
            std::thread::sleep(Duration::from_millis(10));
        }
        fn now_ms(&self) -> u64 {
            self.start.elapsed().as_millis() as u64
        }
    }
    check()?;
    let start = Instant::now();
    run(
        options,
        &mut Native {
            target,
            check,
            start,
            limit: start + Duration::from_millis(u64::from(options.max_duration_ms)),
            button: options.button,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PointerButtonState, WindowGeometry, WindowPointerState};
    struct Fake {
        points: Vec<WindowPointerState>,
        index: usize,
        time: u64,
        rect: WindowRect,
        writes: Vec<WindowRect>,
        fail_after_write: bool,
    }
    impl Backend for Fake {
        fn check(&mut self) -> Result<(), WindowGeometryError> {
            if self.fail_after_write && !self.writes.is_empty() {
                Err(WindowGeometryError::new(
                    "cancelled",
                    "合成取消",
                    "not_started",
                ))
            } else {
                Ok(())
            }
        }
        fn sample(&mut self) -> Result<WindowPointerState, WindowGeometryError> {
            let point = self.points[self.index.min(self.points.len() - 1)];
            self.index += 1;
            Ok(point)
        }
        fn geometry(&mut self) -> Result<WindowGeometry, WindowGeometryError> {
            Ok(WindowGeometry {
                outer_rect: self.rect,
                visible_rect: self.rect,
                monitor_rect: WindowRect {
                    left: 0,
                    top: 0,
                    width: 1000,
                    height: 800,
                },
                work_area: WindowRect {
                    left: 0,
                    top: 0,
                    width: 1000,
                    height: 760,
                },
                primary_screen_width: 2560,
                primary_screen_height: 1440,
                window_dpi: 144,
                maximized: false,
            })
        }
        fn set(&mut self, rect: WindowRect) -> Result<(), WindowGeometryError> {
            self.rect = rect;
            self.writes.push(rect);
            Ok(())
        }
        fn wait(&mut self) {
            self.time += 10;
        }
        fn now_ms(&self) -> u64 {
            self.time
        }
    }
    fn point(x: i32, y: i32, down: bool) -> WindowPointerState {
        WindowPointerState {
            x,
            y,
            left: if down {
                PointerButtonState::Down
            } else {
                PointerButtonState::NotDownOrUnavailable
            },
            right: PointerButtonState::NotDownOrUnavailable,
            middle: PointerButtonState::NotDownOrUnavailable,
        }
    }
    fn options(mode: WindowDragMode) -> WindowDragOptions {
        WindowDragOptions {
            mode,
            button: WindowDragButton::Left,
            max_duration_ms: 100,
            max_updates: 8,
            snap: None,
        }
    }
    fn fake(points: Vec<WindowPointerState>) -> Fake {
        Fake {
            points,
            index: 0,
            time: 0,
            rect: WindowRect {
                left: -800,
                top: 100,
                width: 200,
                height: 100,
            },
            writes: vec![],
            fail_after_write: false,
        }
    }
    #[test]
    fn resize_keeps_baseline_until_either_axis_reaches_ten_then_uses_both_deltas() {
        let mut backend = fake(vec![
            point(0, 0, true),
            point(5, 3, true),
            point(10, 4, true),
            point(10, 4, false),
        ]);
        let result = run(options(WindowDragMode::Resize), &mut backend).unwrap();
        assert_eq!(result.reason, WindowDragStop::ButtonNotDownOrUnavailable);
        assert_eq!(result.accepted_updates, 1);
        assert_eq!(
            backend.writes[0],
            WindowRect {
                left: -800,
                top: 100,
                width: 210,
                height: 104
            }
        );
    }
    #[test]
    fn move_delta_and_cancel_after_partial_submission_never_roll_back() {
        let mut backend = fake(vec![point(0, 0, true), point(20, -10, true)]);
        backend.fail_after_write = true;
        let error = run(options(WindowDragMode::Move), &mut backend).unwrap_err();
        assert_eq!(error.code, "cancelled");
        assert_eq!(error.effect_state, "unknown");
        assert_eq!(backend.writes.len(), 1);
        assert_eq!(backend.rect.left, -780);
        assert_eq!(backend.rect.top, 90);
    }
    #[test]
    fn budgets_stop_without_claiming_button_release_or_drag_completion() {
        let mut backend = fake(vec![point(0, 0, true)]);
        let mut opt = options(WindowDragMode::Move);
        opt.max_updates = 1;
        assert_eq!(
            run(opt, &mut backend).unwrap().reason,
            WindowDragStop::UpdateLimit
        );
        assert_eq!(backend.writes.len(), 1);
        let mut backend = fake(vec![point(0, 0, true)]);
        opt = options(WindowDragMode::Move);
        opt.max_duration_ms = 20;
        assert_eq!(
            run(opt, &mut backend).unwrap().reason,
            WindowDragStop::DurationLimit
        );
        assert_eq!(backend.writes.len(), 1);
    }
    #[test]
    fn left_top_right_share_strict_clock_and_nearness_updates_it_without_a_snap() {
        let opt = WindowSnapOptions {
            bounds: WindowRect {
                left: 0,
                top: 0,
                width: 1000,
                height: 800,
            },
            distance: 30,
            wait_ms: 300,
            initial_elapsed_ms: 0,
        };
        let rect = WindowRect {
            left: 10,
            top: 10,
            width: 200,
            height: 100,
        };
        let mut last = 0;
        assert!(snaps(rect, opt, 300, &mut last).unwrap().is_empty());
        assert_eq!(last, 300);
        let result = snaps(rect, opt, 601, &mut last).unwrap();
        assert_eq!(result, vec![WindowRect { left: 0, ..rect }]);
        assert_eq!(last, 601);
        // 底边不吸附，也不额外等待。
        assert!(snaps(
            WindowRect {
                left: 100,
                top: 690,
                width: 200,
                height: 100
            },
            opt,
            1000,
            &mut last
        )
        .unwrap()
        .is_empty());
    }
}
