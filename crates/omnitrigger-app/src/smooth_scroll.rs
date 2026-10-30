//! 为普通滚动区域提供短时缓出的滚轮动画，不增加布局节点。

use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{
    point, prelude::*, px, AnyElement, App, Bounds, DispatchPhase, Div, Element, ElementId,
    EntityId, GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, LayoutId,
    MouseDownEvent, Pixels, ScrollDelta, ScrollHandle, ScrollWheelEvent, Stateful, Window,
};

const DURATION: Duration = Duration::from_millis(120);
const POSITION_EPSILON: f32 = 0.05;

#[derive(Default)]
pub struct ScrollRegions {
    regions: RefCell<HashMap<&'static str, SmoothState>>,
}

type SmoothState = Rc<RefCell<Region>>;

struct Region {
    handle: ScrollHandle,
    motion: Motion,
    frame_pending: bool,
    paints: u64,
    last_tick_paint: Option<u64>,
}

impl Region {
    fn new(handle: ScrollHandle) -> Self {
        Self {
            handle,
            motion: Motion::default(),
            frame_pending: false,
            paints: 0,
            last_tick_paint: None,
        }
    }

    fn begin_frame(&mut self) -> bool {
        self.frame_pending = false;
        if self.motion.tween.is_none() {
            return false;
        }
        // GPUI 先回调、后绘制；首帧不能按上一轮绘制距今多久来判定隐藏。
        if self.last_tick_paint == Some(self.paints) {
            self.motion.cancel();
            return false;
        }
        self.last_tick_paint = Some(self.paints);
        true
    }
}

impl ScrollRegions {
    pub fn wrap(&self, id: &'static str, element: Stateful<Div>) -> SmoothScroll {
        let state = self
            .regions
            .borrow_mut()
            .entry(id)
            .or_insert_with(|| Rc::new(RefCell::new(Region::new(ScrollHandle::new()))))
            .clone();
        Self::wrap_state(element, state)
    }

    pub fn wrap_with_handle(
        &self,
        id: &'static str,
        element: Stateful<Div>,
        handle: &ScrollHandle,
    ) -> SmoothScroll {
        let state = self
            .regions
            .borrow_mut()
            .entry(id)
            .or_insert_with(|| Rc::new(RefCell::new(Region::new(handle.clone()))))
            .clone();
        state.borrow_mut().handle = handle.clone();
        Self::wrap_state(element, state)
    }

    fn wrap_state(element: Stateful<Div>, state: SmoothState) -> SmoothScroll {
        let handle = state.borrow().handle.clone();
        SmoothScroll {
            inner: element.track_scroll(&handle).into_any_element(),
            state,
        }
    }

    pub fn cancel(&self, id: &'static str) {
        if let Some(state) = self.regions.borrow().get(id) {
            state.borrow_mut().motion.cancel();
            // 已排队的帧仍占有调度权，新输入复用该帧，避免形成两条动画链。
        }
    }

    pub fn cancel_all(&self) {
        for state in self.regions.borrow().values() {
            state.borrow_mut().motion.cancel();
        }
    }
}

pub struct SmoothScroll {
    inner: AnyElement,
    state: SmoothState,
}

impl IntoElement for SmoothScroll {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SmoothScroll {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.inner.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Hitbox {
        // 放在子元素之前，子元素的遮挡区域仍能屏蔽本区域。
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        self.inner.prepaint(window, cx);
        hitbox
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        hitbox: &mut Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        // 只使所属视图失效，保留其他缓存视图；不能在帧回调中取 current_view。
        let owner = window.current_view();
        let line_height = window.line_height();
        self.inner.paint(window, cx);
        {
            let mut state = self.state.borrow_mut();
            let actual = f32::from(state.handle.offset().y);
            state.motion.observe(actual);
            state.paints = state.paints.wrapping_add(1);
        }
        let click_state = self.state.clone();
        let click_hitbox = hitbox.clone();
        window.on_mouse_event(move |_: &MouseDownEvent, phase, window, _| {
            if phase == DispatchPhase::Capture && click_hitbox.is_hovered(window) {
                click_state.borrow_mut().motion.cancel();
            }
        });
        let state = self.state.clone();
        let hitbox = hitbox.clone();
        // Capture 按注册顺序派发。子区域先 paint，因而嵌套时先处理内层；
        // 也能在 GPUI 的默认 Bubble 滚动修改 offset 前消费事件。
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            if phase != DispatchPhase::Capture || !hitbox.should_handle_scroll(window) {
                return;
            }
            let delta = event.delta.pixel_delta(line_height);
            // 这些区域仅垂直滚动，沿用 GPUI 的横向回退规则。
            let delta = f32::from(if delta.y != px(0.) { delta.y } else { delta.x });
            if !delta.is_finite() || delta == 0. {
                return;
            }
            let animated = matches!(event.delta, ScrollDelta::Lines(_));
            let handled = {
                let mut state = state.borrow_mut();
                let offset = state.handle.offset();
                let actual = f32::from(offset.y);
                let limit = f32::from(state.handle.max_offset().height);
                if animated {
                    let handled = state.motion.push(actual, delta, limit, Instant::now());
                    state.last_tick_paint = None;
                    handled
                } else {
                    state.motion.cancel();
                    let next = clamp(actual + delta, limit);
                    if next == actual {
                        false
                    } else {
                        state.handle.set_offset(point(offset.x, px(next)));
                        true
                    }
                }
            };
            if handled {
                cx.stop_propagation();
                if animated {
                    // 输入只合并目标。位移和通知由唯一的下一帧回调完成，
                    // 避免每个滚轮事件先多重绘一次尚未改变的画面。
                    queue_frame(&state, owner, window);
                } else {
                    cx.notify(owner);
                }
            }
        });
    }
}

fn queue_frame(state: &SmoothState, owner: EntityId, window: &Window) {
    {
        let mut state = state.borrow_mut();
        if state.frame_pending || state.motion.tween.is_none() {
            return;
        }
        state.frame_pending = true;
    }
    let weak = Rc::downgrade(state);
    window.on_next_frame(move |window, cx| {
        let Some(state) = weak.upgrade() else {
            return;
        };
        let running = {
            let mut state = state.borrow_mut();
            if !state.begin_frame() {
                return;
            }
            let offset = state.handle.offset();
            let actual = f32::from(offset.y);
            let limit = f32::from(state.handle.max_offset().height);
            let Some(next) = state.motion.step(actual, limit, Instant::now()) else {
                return;
            };
            state.handle.set_offset(point(offset.x, px(next)));
            state.motion.tween.is_some()
        };
        cx.notify(owner);
        if running {
            queue_frame(&state, owner, window);
        }
    });
}

#[derive(Default)]
struct Motion {
    tween: Option<Tween>,
    expected: Option<f32>,
}

struct Tween {
    from: f32,
    target: f32,
    started: Instant,
}

fn clamp(offset: f32, limit: f32) -> f32 {
    offset.clamp(-limit.max(0.), 0.)
}

impl Motion {
    fn cancel(&mut self) {
        self.tween = None;
        self.expected = None;
    }

    fn observe(&mut self, actual: f32) {
        if self
            .expected
            .is_some_and(|expected| (actual - expected).abs() > POSITION_EPSILON)
        {
            // 键盘选中、拖动或调用方定位拥有优先权。
            self.cancel();
        }
    }

    fn push(&mut self, actual: f32, delta: f32, limit: f32, now: Instant) -> bool {
        self.observe(actual);
        if clamp(actual + delta, limit) == actual {
            self.cancel();
            return false;
        }
        let base = self.tween.as_ref().map_or(actual, |tween| {
            if (tween.target - actual).signum() == delta.signum() {
                tween.target
            } else {
                actual
            }
        });
        let target = clamp(base + delta, limit);
        if self
            .tween
            .as_ref()
            .is_some_and(|tween| tween.target == target)
        {
            // 目标已在边界时继续转滚轮，不延后抵达边界的时刻。
            // 否则连续输入会不断重启缓出，外层始终收不到边界事件。
            return true;
        }
        self.tween = Some(Tween {
            from: actual,
            target,
            started: now,
        });
        self.expected = Some(actual);
        true
    }

    fn step(&mut self, actual: f32, limit: f32, now: Instant) -> Option<f32> {
        self.observe(actual);
        let tween = self.tween.as_mut()?;
        tween.target = clamp(tween.target, limit);
        let elapsed = now.saturating_duration_since(tween.started).as_secs_f32();
        let progress = (elapsed / DURATION.as_secs_f32()).min(1.);
        let eased = 1. - (1. - progress).powi(3);
        let next = clamp(tween.from + (tween.target - tween.from) * eased, limit);
        self.expected = Some(next);
        if progress >= 1. {
            self.tween = None;
            self.expected = None;
        }
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_frame_after_idle_runs_before_paint_and_hidden_regions_stop_afterwards() {
        let now = Instant::now();
        let mut region = Region::new(ScrollHandle::new());
        region.paints = 1;
        region.motion.push(0., -100., 500., now);
        // 过了很久没有重绘也必须接纳首次动画回调。
        assert!(region.begin_frame());
        let next = region
            .motion
            .step(0., 500., now + Duration::from_millis(16))
            .unwrap();
        assert!(next < 0. && next > -100.);
        region.paints += 1;
        assert!(region.begin_frame());
        // 请求绘制后区域未再出现，下一回调应停止，不持续刷新隐藏页面。
        assert!(!region.begin_frame());
        assert!(region.motion.tween.is_none());
    }

    #[test]
    fn consecutive_wheel_events_accumulate_target_without_losing_fractional_lines() {
        let now = Instant::now();
        let mut motion = Motion::default();
        assert!(motion.push(0., -30.25, 500., now));
        let actual = motion
            .step(0., 500., now + Duration::from_millis(40))
            .unwrap();
        assert!(actual < 0. && actual > -30.25);
        assert!(motion.push(actual, -30.25, 500., now + Duration::from_millis(40)));
        assert_eq!(motion.tween.as_ref().unwrap().target, -60.5);
        assert_eq!(
            motion.step(actual, 500., now + Duration::from_millis(230)),
            Some(-60.5)
        );
        assert!(motion.tween.is_none());
    }

    #[test]
    fn reversing_discards_the_old_destination_and_moves_from_current_offset() {
        let now = Instant::now();
        let mut motion = Motion::default();
        motion.push(-100., -120., 500., now);
        let actual = motion
            .step(-100., 500., now + Duration::from_millis(50))
            .unwrap();
        assert!(motion.push(actual, 20., 500., now + Duration::from_millis(50)));
        assert_eq!(motion.tween.as_ref().unwrap().target, actual + 20.);
        let next = motion
            .step(actual, 500., now + Duration::from_millis(70))
            .unwrap();
        assert!(next > actual);
    }

    #[test]
    fn boundaries_handoff_only_after_the_inner_area_has_reached_its_edge() {
        let now = Instant::now();
        let mut motion = Motion::default();
        assert!(motion.push(-80., -100., 100., now));
        assert!(motion.push(-80., -100., 100., now));
        assert_eq!(motion.step(-80., 100., now + DURATION), Some(-100.));
        assert!(!motion.push(-100., -1., 100., now + DURATION));
        assert!(!motion.push(0., 1., 100., now + DURATION));
        assert!(!motion.push(0., -1., 0., now + DURATION));
    }

    #[test]
    fn repeated_wheel_events_at_a_clamped_target_do_not_extend_the_animation() {
        let now = Instant::now();
        let mut motion = Motion::default();
        motion.push(-80., -100., 100., now);
        let actual = motion
            .step(-80., 100., now + Duration::from_millis(60))
            .unwrap();
        assert!(motion.push(actual, -100., 100., now + Duration::from_millis(60)));
        assert_eq!(motion.tween.as_ref().unwrap().started, now);
        assert_eq!(motion.step(actual, 100., now + DURATION), Some(-100.));
        assert!(!motion.push(-100., -100., 100., now + DURATION));
    }

    #[test]
    fn a_wheel_step_is_nearly_complete_by_60ms_and_finishes_at_120ms() {
        let now = Instant::now();
        let mut motion = Motion::default();
        motion.push(0., -100., 500., now);
        let halfway = motion
            .step(0., 500., now + Duration::from_millis(60))
            .unwrap();
        assert!((halfway + 87.5).abs() < 0.001);
        assert_eq!(
            motion.step(halfway, 500., now + Duration::from_millis(120)),
            Some(-100.)
        );
        assert!(motion.tween.is_none());
    }

    #[test]
    fn external_positioning_cancels_instead_of_dragging_the_view_back() {
        let now = Instant::now();
        let mut motion = Motion::default();
        motion.push(-20., -100., 500., now);
        let actual = motion
            .step(-20., 500., now + Duration::from_millis(40))
            .unwrap();
        assert!(actual < -20.);
        assert_eq!(
            motion.step(-250., 500., now + Duration::from_millis(60)),
            None
        );
        assert!(motion.tween.is_none());
    }

    #[test]
    fn shorter_content_clamps_the_destination_and_respects_native_repositioning() {
        let now = Instant::now();
        let mut motion = Motion::default();
        motion.push(-20., -300., 500., now);
        assert_eq!(motion.step(-20., 80., now + DURATION), Some(-80.));
        motion.push(-80., 20., 80., now);
        assert_eq!(
            motion.step(-30., 30., now + Duration::from_millis(20)),
            None
        );
    }
}
