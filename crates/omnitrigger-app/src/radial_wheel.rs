//! 自定义轮盘只负责方向选择与请求；动作执行、前台上下文和生命周期由管理界面协调。
use std::{
    f32::consts::TAU,
    sync::mpsc::{SyncSender, TrySendError},
};

use gpui::{
    div, prelude::*, px, rgb, Context, Div, FontWeight, MouseButton, MouseDownEvent,
    MouseMoveEvent, Stateful, Window,
};

use crate::{
    assets::Icon,
    event_bridge::UiWake,
    theme,
    wheels::{WheelAccent, WheelDefinition},
};

pub const WHEEL_SIZE: f32 = 520.;
const CENTER: f32 = WHEEL_SIZE / 2.;
const ORBIT_RADIUS: f32 = 169.;
const DEAD_ZONE: f32 = 64.;
const OUTER_RADIUS: f32 = 236.;
const NODE_WIDTH: f32 = 104.;
const NODE_HEIGHT: f32 = 66.;
const MAX_SLOTS: usize = 8;

#[derive(Debug)]
pub enum WheelRequest {
    Run {
        wheel_id: String,
        slot: usize,
        generation: u64,
    },
    Configure,
    Close,
}

/// 坐标均为轮盘窗口内的逻辑像素，从上方开始、沿顺时针排列。
pub fn slot_position(slot: usize, count: usize) -> Option<(f32, f32)> {
    if !(1..=MAX_SLOTS).contains(&count) || slot >= count {
        return None;
    }
    let angle = slot as f32 * TAU / count as f32;
    Some((
        CENTER + angle.sin() * ORBIT_RADIUS,
        CENTER - angle.cos() * ORBIT_RADIUS,
    ))
}

/// 按方向命中，节点之间的轨道空隙仍可选择；中心、圈外、窗口外及非有限坐标均不选中。
pub fn hit_slot(point: (f32, f32), count: usize) -> Option<usize> {
    let (x, y) = point;
    if !(1..=MAX_SLOTS).contains(&count)
        || !x.is_finite()
        || !y.is_finite()
        || !(0. ..WHEEL_SIZE).contains(&x)
        || !(0. ..WHEEL_SIZE).contains(&y)
    {
        return None;
    }
    let (dx, dy) = (x - CENTER, y - CENTER);
    let radius = dx.hypot(dy);
    if radius <= DEAD_ZONE || radius > OUTER_RADIUS {
        return None;
    }
    let direction = dx.atan2(-dy).rem_euclid(TAU);
    Some(((direction / TAU * count as f32 + 0.5).floor() as usize) % count)
}

fn pointer_slot(point: (f32, f32), count: usize) -> Option<usize> {
    // 标题和说明不承载动作，避免点击文字时落入顶部/底部方向。
    if point.1 < 54. || point.1 > 470. {
        None
    } else {
        hit_slot(point, count)
    }
}

fn ready_at(states: &[Option<String>], count: usize, slot: usize) -> bool {
    slot < count && states.get(slot).is_some_and(Option::is_none)
}

fn next_ready(
    states: &[Option<String>],
    count: usize,
    selected: Option<usize>,
    forward: bool,
) -> Option<usize> {
    if !(1..=MAX_SLOTS).contains(&count) {
        return None;
    }
    let current = selected.filter(|&slot| slot < count);
    for offset in 1..=count {
        let candidate = match current {
            Some(slot) if forward => (slot + offset) % count,
            Some(slot) => (slot + count - offset) % count,
            None => offset - 1,
        };
        if ready_at(states, count, candidate) {
            return Some(candidate);
        }
    }
    None
}

fn window_hwnd(window: &Window) -> Option<isize> {
    raw_window_handle::HasWindowHandle::window_handle(window)
        .ok()
        .and_then(|handle| match handle.as_raw() {
            raw_window_handle::RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
            _ => None,
        })
}

pub struct RadialWheel {
    definition: Option<WheelDefinition>,
    generation: u64,
    states: Vec<Option<String>>,
    requests: SyncSender<WheelRequest>,
    wake: UiWake,
    selected: Option<usize>,
    pointer_selecting: bool,
    visible: bool,
    error: Option<String>,
}

impl RadialWheel {
    pub fn new(
        requests: SyncSender<WheelRequest>,
        wake: UiWake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let view = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            let _ = view.update(cx, |view, _| view.request(WheelRequest::Close, window));
            false
        });
        Self {
            definition: None,
            generation: 0,
            states: Vec::new(),
            requests,
            wake,
            selected: None,
            pointer_selecting: false,
            visible: false,
            error: None,
        }
    }

    pub fn present(
        &mut self,
        definition: WheelDefinition,
        generation: u64,
        states: Vec<Option<String>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.error = (!(1..=MAX_SLOTS).contains(&definition.slots.len()))
            .then(|| "轮盘需要 1–8 个动作，请到配置中调整。".into());
        self.definition = Some(definition);
        self.generation = generation;
        self.states = states;
        self.selected = None;
        self.pointer_selecting = false;
        self.visible = true;
        // 不调用 focus / activate，保留呼出时的前台目标。
        cx.notify();
    }

    pub fn hide(&mut self, window: &mut Window) {
        self.visible = false;
        self.selected = None;
        self.pointer_selecting = false;
        if let Some(hwnd) = window_hwnd(window) {
            omnitrigger_windows::hide_panel_window(hwnd);
        }
        window.refresh();
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn update_states(&mut self, states: Vec<Option<String>>, cx: &mut Context<Self>) {
        if self.states != states {
            self.states = states;
            cx.notify();
        }
    }

    pub fn dismiss_outside(&mut self, window: &mut Window) {
        self.request(WheelRequest::Close, window);
    }

    pub fn dismiss_escape(&mut self, window: &mut Window) {
        self.request(WheelRequest::Close, window);
    }

    pub fn navigation_key(
        &mut self,
        key: omnitrigger_windows::PanelKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use omnitrigger_windows::PanelKey;
        if !self.visible {
            return;
        }
        match key {
            PanelKey::Escape => self.request(WheelRequest::Close, window),
            PanelKey::Enter => {
                if let Some(slot) = self.selected {
                    self.run(slot, window);
                }
            }
            PanelKey::Digit(digit @ 1..=8) => self.run(usize::from(digit - 1), window),
            PanelKey::Digit(_) => {}
            PanelKey::Left | PanelKey::Up | PanelKey::Right | PanelKey::Down => {
                self.selected = next_ready(
                    &self.states,
                    self.count(),
                    self.selected,
                    matches!(key, PanelKey::Right | PanelKey::Down),
                );
                self.pointer_selecting = false;
                self.error = None;
            }
        }
        cx.notify();
    }

    fn count(&self) -> usize {
        self.definition
            .as_ref()
            .map(|definition| definition.slots.len())
            .filter(|count| (1..=MAX_SLOTS).contains(count))
            .unwrap_or(0)
    }

    fn color(&self) -> u32 {
        match self
            .definition
            .as_ref()
            .map(|definition| &definition.accent)
        {
            Some(WheelAccent::Violet) => 0xb5a0ff,
            Some(WheelAccent::Amber) => 0xffc36a,
            Some(WheelAccent::Mint) | None => 0x5df3bd,
        }
    }

    fn unavailable(&self, slot: usize) -> Option<String> {
        if slot >= self.count() {
            return Some("此位置没有动作。".into());
        }
        match self.states.get(slot) {
            Some(reason) => reason.clone(),
            None => Some("动作状态尚未同步，请重新打开轮盘。".into()),
        }
    }

    fn run(&mut self, slot: usize, window: &mut Window) {
        if !self.visible || slot >= self.count() {
            return;
        }
        self.selected = Some(slot);
        if let Some(reason) = self.unavailable(slot) {
            self.error = Some(reason);
            window.refresh();
            return;
        }
        let Some(definition) = &self.definition else {
            return;
        };
        self.request(
            WheelRequest::Run {
                wheel_id: definition.id.clone(),
                slot,
                generation: self.generation,
            },
            window,
        );
    }

    fn request(&mut self, request: WheelRequest, window: &mut Window) {
        if !self.visible {
            return;
        }
        match self.requests.try_send(request) {
            Ok(()) => {
                // 接收方在同一 UI 线程，唤醒前先同步隐藏，确保执行动作时画面已移除。
                // SyncSender 没有容量预留 API；成功入队后隐藏也能保证 Full 时保留错误界面。
                self.hide(window);
                self.wake.notify();
            }
            Err(TrySendError::Full(_)) => {
                self.error = Some("请求队列繁忙，请稍后重试。".into());
                window.refresh();
            }
            Err(TrySendError::Disconnected(_)) => {
                self.error = Some("轮盘管理已断开，请重新打开程序。".into());
                window.refresh();
            }
        }
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        // GPUI 的 event.position 已是窗口内逻辑像素；不再乘除 DPI。
        let selected = pointer_slot(
            (event.position.x.into(), event.position.y.into()),
            self.count(),
        );
        if self.selected != selected || !self.pointer_selecting {
            self.selected = selected;
            self.pointer_selecting = true;
            self.error = None;
            cx.notify();
        }
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        cx.stop_propagation();
        if event.button == MouseButton::Right {
            self.request(WheelRequest::Close, window);
        } else if event.button == MouseButton::Left {
            match pointer_slot(
                (event.position.x.into(), event.position.y.into()),
                self.count(),
            ) {
                Some(slot) => self.run(slot, window),
                None => self.request(WheelRequest::Close, window),
            }
        }
        cx.notify();
    }

    fn node(&self, slot: usize) -> Div {
        let (x, y) = slot_position(slot, self.count()).expect("只绘制有效槽位");
        let definition = self.definition.as_ref().expect("槽位来自有效轮盘");
        let selected = self.selected == Some(slot);
        let ready = self.unavailable(slot).is_none();
        let color = self.color();
        div()
            .absolute()
            .left(px(x - NODE_WIDTH / 2.))
            .top(px(y - NODE_HEIGHT / 2.))
            .w(px(NODE_WIDTH))
            .h(px(NODE_HEIGHT))
            .p(px(8.))
            .rounded(px(15.))
            .bg(theme::gradient(
                145.,
                theme::hsla(
                    if selected { color } else { 0x222c41 },
                    if selected { 0.19 } else { 0.8 },
                ),
                theme::hsla(0x0c111b, 1.),
            ))
            .border_1()
            .border_color(theme::hsla(
                if selected { color } else { 0x6e819e },
                if selected { 0.9 } else { 0.26 },
            ))
            .when(selected, |node| node.shadow(theme::glow(color, 0.2, 22.)))
            .when(ready, |node| node.cursor_pointer())
            .flex()
            .flex_col()
            .gap(px(3.))
            .child(
                div()
                    .h(px(17.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .font_family(theme::MONO_FONT)
                            .text_size(px(10.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme::hsla(
                                if ready { color } else { theme::TEXT_DIM },
                                if selected { 1. } else { 0.7 },
                            ))
                            .child(format!("{:02}", slot + 1)),
                    )
                    .child(theme::icon(
                        if ready { Icon::Bolt } else { Icon::Alert },
                        11.,
                        if ready { color } else { theme::RECORD },
                    )),
            )
            .child(
                div()
                    .min_w_0()
                    .line_clamp(2)
                    .text_ellipsis()
                    .text_size(px(11.5))
                    .line_height(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(if ready { theme::TEXT } else { theme::TEXT_DIM }))
                    .child(definition.slots[slot].label.clone()),
            )
    }

    fn center(&self) -> Div {
        let color = self.color();
        let slot = self.selected.filter(|&slot| slot < self.count());
        let title = slot.and_then(|slot| {
            self.definition
                .as_ref()
                .map(|definition| definition.slots[slot].label.clone())
        });
        div()
            .absolute()
            .left(px(CENTER - DEAD_ZONE))
            .top(px(CENTER - DEAD_ZONE))
            .size(px(DEAD_ZONE * 2.))
            .rounded_full()
            .bg(rgb(0x0b1019))
            .border_1()
            .border_color(theme::hsla(color, if slot.is_some() { 0.35 } else { 0.13 }))
            .shadow(theme::glow(color, 0.06, 35.))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(5.))
            .child(
                div()
                    .font_family(theme::MONO_FONT)
                    .text_size(px(if slot.is_some() { 26. } else { 19. }))
                    .text_color(theme::hsla(color, 0.9))
                    .child(
                        slot.map(|slot| format!("{:02}", slot + 1))
                            .unwrap_or_else(|| "ORBIT".into()),
                    ),
            )
            .child(
                div()
                    .w(px(106.))
                    .line_clamp(2)
                    .text_ellipsis()
                    .text_center()
                    .text_size(px(11.5))
                    .line_height(px(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::TEXT_SOFT))
                    .child(title.unwrap_or_else(|| "向外指向动作".into())),
            )
            .child(
                div()
                    .text_size(px(9.5))
                    .text_color(rgb(theme::TEXT_DIM))
                    .child("点击中央取消"),
            )
    }

    fn tool(&self, id: &'static str, glyph: Icon) -> Stateful<Div> {
        div()
            .id(id)
            .size(px(30.))
            .rounded(px(10.))
            .border_1()
            .border_color(theme::hsla(self.color(), 0.16))
            .bg(theme::hsla(0x182132, 0.7))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|style| style.bg(rgb(0x243047)))
            .child(theme::icon(glyph, 14., theme::TEXT_MUTED))
    }
}

fn orbit_dot(x: f32, y: f32, size: f32, color: u32, alpha: f32) -> Div {
    div()
        .absolute()
        .left(px(x - size / 2.))
        .top(px(y - size / 2.))
        .size(px(size))
        .rounded_full()
        .bg(theme::hsla(color, alpha))
}

impl Render for RadialWheel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.pointer_selecting && !window.is_window_hovered() {
            self.selected = None;
        }
        let color = self.color();
        let count = self.count();
        let mut track = div()
            .absolute()
            .left(px(0.))
            .top(px(0.))
            .size(px(WHEEL_SIZE));
        for (radius, alpha) in [(ORBIT_RADIUS, 0.22), (ORBIT_RADIUS - 27., 0.07)] {
            track = track.child(
                div()
                    .absolute()
                    .left(px(CENTER - radius))
                    .top(px(CENTER - radius))
                    .size(px(radius * 2.))
                    .rounded_full()
                    .border_1()
                    .border_color(theme::hsla(color, alpha)),
            );
        }
        for tick in 0..64 {
            let angle = tick as f32 * TAU / 64.;
            track = track.child(orbit_dot(
                CENTER + angle.sin() * 215.,
                CENTER - angle.cos() * 215.,
                if tick % 8 == 0 { 3. } else { 1.5 },
                color,
                if tick % 8 == 0 { 0.3 } else { 0.12 },
            ));
        }
        if let Some(slot) = self.selected.filter(|&slot| slot < count) {
            let angle = slot as f32 * TAU / count as f32;
            for step in 0..8 {
                let radius = 77. + step as f32 * 7.;
                track = track.child(orbit_dot(
                    CENTER + angle.sin() * radius,
                    CENTER - angle.cos() * radius,
                    2.5,
                    color,
                    0.35 + step as f32 * 0.06,
                ));
            }
            for tick in -3..=3 {
                let arc = angle + tick as f32 * 0.045;
                track = track.child(orbit_dot(
                    CENTER + arc.sin() * 215.,
                    CENTER - arc.cos() * 215.,
                    3.,
                    color,
                    0.85,
                ));
            }
        }
        let name = self
            .definition
            .as_ref()
            .map(|definition| definition.name.clone())
            .unwrap_or_else(|| "自定义轮盘".into());
        let status = self
            .error
            .clone()
            .or_else(|| self.selected.and_then(|slot| self.unavailable(slot)));
        div()
            .id("radial-wheel")
            .relative()
            .w(px(WHEEL_SIZE))
            .h(px(WHEEL_SIZE))
            .overflow_hidden()
            .rounded(px(27.))
            .border_1()
            .border_color(theme::hsla(color, 0.24))
            .bg(theme::gradient(
                135.,
                theme::hsla(0x101828, 0.99),
                theme::hsla(0x080c14, 0.99),
            ))
            .font_family(theme::UI_FONT)
            .text_color(rgb(theme::TEXT))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::mouse_down))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if !*hovered && this.pointer_selecting && this.selected.take().is_some() {
                    cx.notify();
                }
            }))
            .child(track)
            .children((0..count).map(|slot| self.node(slot)))
            .child(self.center())
            .child(
                div()
                    .absolute()
                    .left(px(22.))
                    .right(px(22.))
                    .top(px(15.))
                    .h(px(30.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| cx.stop_propagation()),
                    )
                    .child(self.tool("wheel-configure", Icon::Sliders).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            this.request(WheelRequest::Configure, window);
                            cx.notify();
                        }),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_center()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme::TEXT_SOFT))
                            .child(name),
                    )
                    .child(self.tool("wheel-close", Icon::Close).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            this.request(WheelRequest::Close, window);
                            cx.notify();
                        }),
                    )),
            )
            .child(
                div()
                    .absolute()
                    .left(px(24.))
                    .right(px(24.))
                    .bottom(px(13.))
                    .h(px(30.))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| cx.stop_propagation()),
                    )
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_center()
                            .line_clamp(2)
                            .text_ellipsis()
                            .text_color(rgb(if status.is_some() {
                                theme::RECORD_TEXT
                            } else {
                                theme::TEXT_DIM
                            }))
                            .child(status.unwrap_or_else(|| {
                                "指向后点击 · 1–8 直达 · 方向键 + Enter · Esc / 右键取消".into()
                            })),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eight_orbit_centers_match_clockwise_directions_and_nodes_do_not_overlap() {
        let positions: Vec<_> = (0..8).map(|slot| slot_position(slot, 8).unwrap()).collect();
        for (slot, &point) in positions.iter().enumerate() {
            assert_eq!(hit_slot(point, 8), Some(slot));
            for &(other_x, other_y) in &positions[slot + 1..] {
                assert!(
                    (point.0 - other_x).abs() >= NODE_WIDTH
                        || (point.1 - other_y).abs() >= NODE_HEIGHT
                );
            }
        }
        assert_eq!(hit_slot((CENTER, CENTER - 100.), 8), Some(0));
        assert_eq!(hit_slot((CENTER + 100., CENTER), 8), Some(2));
        assert_eq!(hit_slot((CENTER, CENTER + 100.), 8), Some(4));
        assert_eq!(hit_slot((CENTER - 100., CENTER), 8), Some(6));
    }

    #[test]
    fn dead_zone_circle_and_window_boundaries_are_safe() {
        for point in [
            (CENTER, CENTER),
            (CENTER + DEAD_ZONE, CENTER),
            (-1., CENTER),
            (WHEEL_SIZE, CENTER),
            (CENTER, CENTER + OUTER_RADIUS + 0.01),
            (0., 0.),
            (f32::NAN, CENTER),
            (CENTER, f32::INFINITY),
        ] {
            assert_eq!(hit_slot(point, 8), None, "{point:?}");
        }
        assert_eq!(hit_slot((CENTER + DEAD_ZONE + 0.01, CENTER), 8), Some(2));
        assert_eq!(hit_slot((CENTER + OUTER_RADIUS, CENTER), 8), Some(2));
        assert_eq!(hit_slot((CENTER, CENTER - 100.), 0), None);
        assert_eq!(hit_slot((CENTER, CENTER - 100.), 9), None);
        assert!(slot_position(8, 8).is_none());
        assert_eq!(pointer_slot((CENTER, 35.), 8), None);
        assert_eq!(pointer_slot((CENTER, 485.), 8), None);
    }

    #[test]
    fn direction_sectors_have_deterministic_clockwise_boundaries() {
        let boundary = TAU / 16.;
        for (angle, slot) in [
            (boundary - 0.0001, 0),
            (boundary + 0.0001, 1),
            (-boundary + 0.0001, 0),
            (-boundary - 0.0001, 7),
        ] {
            assert_eq!(
                hit_slot(
                    (CENTER + angle.sin() * 120., CENTER - angle.cos() * 120.),
                    8
                ),
                Some(slot)
            );
        }
    }

    #[test]
    fn keyboard_navigation_cycles_only_through_available_slots() {
        let states = vec![None, Some("不可用".into()), None, Some("执行中".into())];
        assert_eq!(next_ready(&states, 4, None, true), Some(0));
        assert_eq!(next_ready(&states, 4, Some(0), true), Some(2));
        assert_eq!(next_ready(&states, 4, Some(2), true), Some(0));
        assert_eq!(next_ready(&states, 4, Some(0), false), Some(2));
        assert!(!ready_at(&states, 4, 1));
        assert!(!ready_at(&states, 4, 4));
        assert!(!ready_at(&[], 4, 0));
        assert_eq!(next_ready(&[Some("不可用".into())], 1, None, true), None);
        assert_eq!(next_ready(&[], 4, None, false), None);
    }
}
