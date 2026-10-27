//! 自定义轮盘只负责方向选择与请求；动作执行、前台上下文和生命周期由管理界面协调。
use std::{
    cell::Cell,
    f32::consts::{PI, TAU},
    rc::Rc,
    sync::mpsc::{SyncSender, TrySendError},
};

use gpui::{
    canvas, div, point, prelude::*, px, rgb, Background, Bounds, BoxShadow, Context, Div,
    FocusHandle, FontWeight, Hsla, KeyDownEvent, Keystroke, MouseButton, MouseDownEvent,
    MouseMoveEvent, PathBuilder, Pixels, Point, Stateful, Window,
};

use crate::{
    assets::Icon,
    event_bridge::UiWake,
    theme,
    wheels::{WheelAccent, WheelDefinition, WheelPosition},
};

pub const WHEEL_SIZE: f32 = 520.;
const CENTER: f32 = WHEEL_SIZE / 2.;
const ORBIT_RADIUS: f32 = 169.;
const DEAD_ZONE: f32 = 64.;
const OUTER_RADIUS: f32 = 236.;
const NODE_WIDTH: f32 = 104.;
const NODE_HEIGHT: f32 = 66.;
const LEGACY_MAX_SLOTS: usize = 8;
const MAX_SLOTS: usize = 48;
const EXPLICIT_SIZE: f32 = 840.;
const EXPLICIT_RADII: [f32; 3] = [122., 242., 362.];
const EXPLICIT_OUTER_RADIUS: f32 = 412.;

// 绘制半径只描述外观；命中仍由 `DEAD_ZONE`、`ring_for_radius` 与上下沿排除区决定，
// 扇区画在命中范围之内，不显示指向后却无法选中的区域。
const HUB_RADIUS: f32 = 58.;
const POINTER_RADIUS: f32 = 66.;
const SECTOR_GAP: f32 = 4.;
const LEGACY_BAND: (f32, f32) = (76., 204.);
const LEGACY_DISC: f32 = 214.;
const LEGACY_BEZEL: (f32, f32) = (206.5, 211.);
const EXPLICIT_BANDS: [(f32, f32); 3] = [(74., 176.), (188., 296.), (308., 378.)];
const EXPLICIT_DISC: f32 = 396.;
const EXPLICIT_BEZEL: (f32, f32) = (384.5, 390.);
const UNAVAILABLE: u32 = 0x8a93a8;

/// 浮窗工具并排停靠在轮盘上沿正中，独立命中，避免透明角落形成一整条不可见标题栏。
#[derive(Clone, Copy)]
struct WheelChrome {
    configure: (f32, f32, f32, f32),
    close: (f32, f32, f32, f32),
}

fn wheel_chrome(definition: &WheelDefinition) -> WheelChrome {
    chrome_at(wheel_size(definition) / 2.)
}

fn chrome_at(center: f32) -> WheelChrome {
    WheelChrome {
        configure: (center - 58., 6., 32., 32.),
        close: (center - 18., 6., 76., 32.),
    }
}

fn inside_rect(point: (f32, f32), bounds: (f32, f32, f32, f32)) -> bool {
    let (x, y, width, height) = bounds;
    point.0 >= x && point.0 < x + width && point.1 >= y && point.1 < y + height
}

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
    if !(1..=LEGACY_MAX_SLOTS).contains(&count) || slot >= count {
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
    if !(1..=LEGACY_MAX_SLOTS).contains(&count)
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

/// 拖拽使用固定按下点的物理坐标向量，不依赖轮盘位置、窗口尺寸或显示缩放。
/// 起点取消半径含边界；半径外没有最远距离限制，从正上方开始顺时针分槽。
pub fn drag_slot(
    origin: (i32, i32),
    position: (i32, i32),
    count: usize,
    cancel_radius_px: u32,
) -> Option<usize> {
    if !(1..=LEGACY_MAX_SLOTS).contains(&count) {
        return None;
    }
    // 先扩展再相减；两端为 i32 极值时也不会溢出或因平方超出 i64。
    let dx = (i64::from(position.0) - i64::from(origin.0)) as f64;
    let dy = (i64::from(position.1) - i64::from(origin.1)) as f64;
    if dx.hypot(dy) <= f64::from(cancel_radius_px) {
        return None;
    }
    let direction = dx.atan2(-dy).rem_euclid(std::f64::consts::TAU);
    Some(((direction / std::f64::consts::TAU * count as f64 + 0.5).floor() as usize) % count)
}

fn explicit_layout(definition: &WheelDefinition) -> bool {
    definition.slots.iter().any(|slot| slot.position.is_some())
}

pub fn wheel_size(definition: &WheelDefinition) -> f32 {
    if explicit_layout(definition) {
        EXPLICIT_SIZE
    } else {
        WHEEL_SIZE
    }
}

fn layout_valid(definition: &WheelDefinition) -> bool {
    let count = definition.slots.len();
    if !explicit_layout(definition) {
        return (1..=LEGACY_MAX_SLOTS).contains(&count);
    }
    if !(1..=MAX_SLOTS).contains(&count) {
        return false;
    }
    for (index, slot) in definition.slots.iter().enumerate() {
        let Some(position) = slot.position.as_ref() else {
            return false;
        };
        if position.ring > 2
            || !(1..=16).contains(&position.sectors)
            || position.sector >= position.sectors
        {
            return false;
        }
        for other in &definition.slots[..index] {
            let Some(other) = other.position.as_ref() else {
                return false;
            };
            if other.ring == position.ring
                && (other.sectors != position.sectors
                    || other.half_step != position.half_step
                    || other.sector == position.sector)
            {
                return false;
            }
        }
    }
    true
}

fn position_angle(position: &WheelPosition) -> f32 {
    (f32::from(position.sector) + if position.half_step { 0.5 } else { 0. }) * TAU
        / f32::from(position.sectors)
}

/// 槽位的命中角度范围，与 `hit_slot` / `sparse_vector_slot` 的分槽边界一致。
fn slot_span(definition: &WheelDefinition, slot: usize) -> Option<(f32, f32)> {
    let (middle, sectors) = match definition.slots.get(slot)?.position.as_ref() {
        Some(position) => (position_angle(position), f32::from(position.sectors)),
        None => {
            let count = definition.slots.len() as f32;
            (slot as f32 * TAU / count, count)
        }
    };
    Some((middle - PI / sectors, middle + PI / sectors))
}

fn slot_band(definition: &WheelDefinition, slot: usize) -> Option<(f32, f32)> {
    match definition.slots.get(slot)?.position.as_ref() {
        Some(position) => EXPLICIT_BANDS.get(usize::from(position.ring)).copied(),
        None => Some(LEGACY_BAND),
    }
}

fn ring_for_radius(radius: f64) -> u8 {
    if radius < 182. {
        0
    } else if radius < 302. {
        1
    } else {
        2
    }
}

fn sparse_vector_slot(dx: f64, dy: f64, definition: &WheelDefinition) -> Option<usize> {
    let ring = ring_for_radius(dx.hypot(dy));
    let angle = dx.atan2(-dy).rem_euclid(std::f64::consts::TAU);
    definition
        .slots
        .iter()
        .enumerate()
        .find_map(|(index, slot)| {
            let position = slot.position.as_ref()?;
            if position.ring != ring {
                return None;
            }
            let offset = if position.half_step { 0.5 } else { 0. };
            let sector = ((angle / std::f64::consts::TAU * f64::from(position.sectors) - offset
                + 0.5)
                .floor() as usize)
                % usize::from(position.sectors);
            (sector == usize::from(position.sector)).then_some(index)
        })
}

/// 显式布局按环与固定方位命中；空方位保持为空，不按已有动作数量重新均分。
pub fn hit_slot_for_definition(point: (f32, f32), definition: &WheelDefinition) -> Option<usize> {
    if !layout_valid(definition) {
        return None;
    }
    if !explicit_layout(definition) {
        return hit_slot(point, definition.slots.len());
    }
    let (x, y) = point;
    if !x.is_finite()
        || !y.is_finite()
        || !(0. ..EXPLICIT_SIZE).contains(&x)
        || !(0. ..EXPLICIT_SIZE).contains(&y)
    {
        return None;
    }
    let center = EXPLICIT_SIZE / 2.;
    let (dx, dy) = (f64::from(x - center), f64::from(y - center));
    let radius = dx.hypot(dy);
    if radius <= f64::from(DEAD_ZONE) || radius > f64::from(EXPLICIT_OUTER_RADIUS) {
        return None;
    }
    sparse_vector_slot(dx, dy, definition)
}

/// 拖拽以固定物理起点和冻结的显示缩放选圈；最外环延伸至无穷远，回到物理取消半径内取消。
pub fn drag_slot_for_definition(
    origin: (i32, i32),
    position: (i32, i32),
    definition: &WheelDefinition,
    cancel_radius_px: u32,
    scale: f32,
) -> Option<usize> {
    if !layout_valid(definition) {
        return None;
    }
    if !explicit_layout(definition) {
        return drag_slot(origin, position, definition.slots.len(), cancel_radius_px);
    }
    if !scale.is_finite() || scale <= 0. {
        return None;
    }
    let dx = (i64::from(position.0) - i64::from(origin.0)) as f64;
    let dy = (i64::from(position.1) - i64::from(origin.1)) as f64;
    if dx.hypot(dy) <= f64::from(cancel_radius_px) {
        return None;
    }
    sparse_vector_slot(dx / f64::from(scale), dy / f64::from(scale), definition)
}

fn definition_slot_position(
    definition: &WheelDefinition,
    slot: usize,
) -> Option<(f32, f32, f32, f32)> {
    let item = definition.slots.get(slot)?;
    let Some(position) = item.position.as_ref() else {
        let (x, y) = slot_position(slot, definition.slots.len())?;
        return Some((x, y, NODE_WIDTH, NODE_HEIGHT));
    };
    let radius = *EXPLICIT_RADII.get(usize::from(position.ring))?;
    let angle = position_angle(position);
    let center = EXPLICIT_SIZE / 2.;
    let (width, height) = if position.ring == 0 && position.sectors > 12 {
        (36., 28.)
    } else if position.ring == 0 && position.sectors > 8 {
        (54., 34.)
    } else {
        (84., 50.)
    };
    Some((
        center + angle.sin() * radius,
        center - angle.cos() * radius,
        width,
        height,
    ))
}

#[cfg(test)]
fn slot_position_for_definition(slot: usize, definition: &WheelDefinition) -> Option<(f32, f32)> {
    if !layout_valid(definition) {
        return None;
    }
    definition_slot_position(definition, slot).map(|(x, y, _, _)| (x, y))
}

fn pointer_slot(point: (f32, f32), count: usize) -> Option<usize> {
    // 标题和说明不承载动作，避免点击文字时落入顶部/底部方向。
    if point.1 < 54. || point.1 > 470. {
        None
    } else {
        hit_slot(point, count)
    }
}

fn definition_pointer_slot(point: (f32, f32), definition: &WheelDefinition) -> Option<usize> {
    if !layout_valid(definition) {
        return None;
    }
    let chrome = wheel_chrome(definition);
    if [chrome.configure, chrome.close]
        .into_iter()
        .any(|bounds| inside_rect(point, bounds))
    {
        return None;
    }
    if !explicit_layout(definition) {
        return pointer_slot(point, definition.slots.len());
    }
    if point.1 < 32. || point.1 > EXPLICIT_SIZE - 32. {
        return None;
    }
    // 矩形节点边角可能略跨角度分界；点击实际绘制的节点仍指向这个节点。
    if let Some(slot) = (0..definition.slots.len()).find(|&slot| {
        definition_slot_position(definition, slot).is_some_and(|(x, y, width, height)| {
            (point.0 - x).abs() <= width / 2. && (point.1 - y).abs() <= height / 2.
        })
    }) {
        return Some(slot);
    }
    hit_slot_for_definition(point, definition)
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

/// 本地窗口按键与 Hook 转发共用导航语义；修饰组合保留给原有快捷键。
fn local_key(keystroke: &Keystroke, is_held: bool) -> Option<omnitrigger_windows::PanelKey> {
    use omnitrigger_windows::PanelKey;
    let modifiers = &keystroke.modifiers;
    if modifiers.shift
        || modifiers.control
        || modifiers.alt
        || modifiers.platform
        || modifiers.function
    {
        return None;
    }
    let key = match keystroke.key.as_str() {
        "escape" => PanelKey::Escape,
        "enter" => PanelKey::Enter,
        "left" => PanelKey::Left,
        "right" => PanelKey::Right,
        "up" => PanelKey::Up,
        "down" => PanelKey::Down,
        key => match key.as_bytes() {
            [digit @ b'1'..=b'8'] => PanelKey::Digit(digit - b'0'),
            _ => return None,
        },
    };
    // 允许方向键长按移动，执行键只接受新的按下边沿。
    if is_held && matches!(key, PanelKey::Digit(_) | PanelKey::Enter) {
        None
    } else {
        Some(key)
    }
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
    focus: FocusHandle,
    selected: Option<usize>,
    pointer_selecting: bool,
    drag_origin: Option<(i32, i32)>,
    drag_cancel_radius_px: u32,
    drag_scale: f32,
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
            focus: cx.focus_handle(),
            selected: None,
            pointer_selecting: false,
            drag_origin: None,
            drag_cancel_radius_px: 0,
            drag_scale: 1.,
            visible: false,
            error: None,
        }
    }

    pub fn present(
        &mut self,
        definition: WheelDefinition,
        generation: u64,
        states: Vec<Option<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.error = (!layout_valid(&definition)).then(|| {
            "轮盘布局无效：均分布局需要 1–8 个动作，显式布局最多 48 个且位置不能重复。".into()
        });
        self.definition = Some(definition);
        self.generation = generation;
        self.states = states;
        self.selected = None;
        self.pointer_selecting = false;
        self.drag_origin = None;
        self.drag_cancel_radius_px = 0;
        self.drag_scale = 1.;
        self.visible = true;
        // 只登记 GPUI 视图焦点；不请求原生窗口激活，保留呼出时的前台目标。
        window.focus(&self.focus);
        cx.notify();
    }

    pub fn hide(&mut self, window: &mut Window) {
        self.visible = false;
        self.selected = None;
        self.pointer_selecting = false;
        self.drag_origin = None;
        self.drag_cancel_radius_px = 0;
        self.drag_scale = 1.;
        if let Some(hwnd) = window_hwnd(window) {
            omnitrigger_windows::hide_panel_window(hwnd);
        }
        window.refresh();
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// 原按压被核心认领后进入拖拽模式，初始方向仍以原物理按下点为准。
    pub fn begin_drag(
        &mut self,
        origin: (i32, i32),
        cancel_radius_px: u32,
        position: (i32, i32),
        scale: f32,
        cx: &mut Context<Self>,
    ) {
        if !self.visible {
            return;
        }
        let selected = self.definition.as_ref().and_then(|definition| {
            drag_slot_for_definition(origin, position, definition, cancel_radius_px, scale)
        });
        let changed = self.drag_origin != Some(origin)
            || self.drag_cancel_radius_px != cancel_radius_px
            || self.selected != selected
            || self.drag_scale != scale
            || self.pointer_selecting;
        self.drag_origin = Some(origin);
        self.drag_cancel_radius_px = cancel_radius_px;
        self.drag_scale = scale;
        self.pointer_selecting = false;
        self.selected = selected;
        if changed {
            cx.notify();
        }
    }

    /// 位置来自后端物理坐标快照；窗口内移动、移出窗口及 DPI 不改变选槽基准。
    pub fn update_drag(&mut self, position: (i32, i32), cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        let Some(origin) = self.drag_origin else {
            return;
        };
        let selected = self.definition.as_ref().and_then(|definition| {
            drag_slot_for_definition(
                origin,
                position,
                definition,
                self.drag_cancel_radius_px,
                self.drag_scale,
            )
        });
        if self.selected != selected {
            self.selected = selected;
            self.error = None;
            cx.notify();
        }
    }

    pub fn update_states(&mut self, states: Vec<Option<String>>, cx: &mut Context<Self>) {
        if self.states != states {
            self.states = states;
            cx.notify();
        }
    }

    pub fn dismiss_outside(&mut self, window: &mut Window) {
        if self.drag_origin.is_none() {
            self.request(WheelRequest::Close, window);
        }
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
        if self.drag_origin.is_some() {
            if matches!(key, PanelKey::Escape) {
                self.request(WheelRequest::Close, window);
            }
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

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        let Some(key) = local_key(&event.keystroke, event.is_held) else {
            return;
        };
        cx.stop_propagation();
        self.navigation_key(key, window, cx);
    }

    fn count(&self) -> usize {
        self.definition
            .as_ref()
            .filter(|definition| layout_valid(definition))
            .map(|definition| definition.slots.len())
            .unwrap_or(0)
    }

    fn color(&self) -> u32 {
        accent_color(
            self.definition
                .as_ref()
                .map(|definition| definition.accent)
                .unwrap_or_default(),
        )
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
        if !self.visible || self.drag_origin.is_some() || slot >= self.count() {
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
        if !self.visible || self.drag_origin.is_some() {
            return;
        }
        // GPUI 的 event.position 已是窗口内逻辑像素；不再乘除 DPI。
        let selected = self.definition.as_ref().and_then(|definition| {
            definition_pointer_slot(
                (event.position.x.into(), event.position.y.into()),
                definition,
            )
        });
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
        if self.drag_origin.is_some() {
            return;
        }
        if event.button == MouseButton::Right {
            self.request(WheelRequest::Close, window);
        } else if event.button == MouseButton::Left {
            match self.definition.as_ref().and_then(|definition| {
                definition_pointer_slot(
                    (event.position.x.into(), event.position.y.into()),
                    definition,
                )
            }) {
                Some(slot) => self.run(slot, window),
                None => self.request(WheelRequest::Close, window),
            }
        }
        cx.notify();
    }

    /// 选中扇区的主色：可用动作沿用轮盘配色，不可用动作退为灰色。
    fn tone(&self, slot: usize) -> u32 {
        if self.unavailable(slot).is_none() {
            self.color()
        } else {
            UNAVAILABLE
        }
    }

    fn art(&self) -> WheelArt {
        wheel_art(
            self.definition.as_ref(),
            self.color(),
            self.selected,
            |slot| self.unavailable(slot).is_none(),
        )
    }

    /// 扇区内的文字层；节点矩形沿用命中几何，只是不再绘制卡片底板。
    fn slot_label(&self, slot: usize) -> Div {
        let definition = self.definition.as_ref().expect("槽位来自有效轮盘");
        let (_, _, width, height) =
            definition_slot_position(definition, slot).expect("只绘制有效槽位");
        let (x, y) = label_center(definition, slot).expect("只绘制有效槽位");
        let legacy = definition.slots[slot].position.is_none();
        let dense = width < 84.;
        let selected = self.selected == Some(slot);
        let ready = self.unavailable(slot).is_none();
        let color = self.color();
        div()
            .absolute()
            .left(px(x - width / 2.))
            .top(px(y - height / 2.))
            .w(px(width))
            .h(px(height))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(if dense { 1. } else { 3. }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        div()
                            .font_family(theme::MONO_FONT)
                            .text_size(px(if dense { 8. } else { 10. }))
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme::hsla(
                                if ready { color } else { theme::TEXT_DIM },
                                if selected { 1. } else { 0.72 },
                            ))
                            .child(format!("{:02}", slot + 1)),
                    )
                    .when(!dense, |header| {
                        header.child(theme::icon(
                            if ready { Icon::Bolt } else { Icon::Alert },
                            10.,
                            match (ready, selected) {
                                (false, _) => theme::RECORD,
                                (true, true) => color,
                                (true, false) => theme::TEXT_FAINT,
                            },
                        ))
                    }),
            )
            .child(
                div()
                    .max_w(px(width - 6.))
                    .line_clamp(if legacy { 2 } else { 1 })
                    .text_ellipsis()
                    .text_center()
                    .text_size(px(if dense {
                        9.5
                    } else if legacy {
                        12.5
                    } else {
                        11.5
                    }))
                    .line_height(px(if dense {
                        11.
                    } else if legacy {
                        15.
                    } else {
                        14.
                    }))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(match (ready, selected) {
                        (false, _) => theme::TEXT_DIM,
                        (true, true) => theme::TEXT,
                        (true, false) => theme::TEXT_SOFT,
                    }))
                    .child(definition.slots[slot].label.clone()),
            )
    }

    /// 中心安全区：未选中时显示轮盘名称，选中后显示编号、动作名与确认方式。
    fn hub(&self, center: f32) -> Div {
        let slot = self.selected.filter(|&slot| slot < self.count());
        let tone = slot.map_or(self.color(), |slot| self.tone(slot));
        let dragging = self.drag_origin.is_some();
        let (caption, title, hint) = match slot {
            Some(slot) => (
                format!("{:02}", slot + 1),
                self.definition
                    .as_ref()
                    .map(|definition| definition.slots[slot].label.clone())
                    .unwrap_or_default(),
                if self.unavailable(slot).is_some() {
                    "暂不可用"
                } else if dragging {
                    "松开鼠标键执行"
                } else {
                    "点击执行"
                },
            ),
            None => (
                "ORBIT".into(),
                self.definition
                    .as_ref()
                    .map(|definition| definition.name.clone())
                    .unwrap_or_else(|| "自定义轮盘".into()),
                if dragging {
                    "回到起点取消"
                } else {
                    "点击中央取消"
                },
            ),
        };
        div()
            .absolute()
            .left(px(center - HUB_RADIUS))
            .top(px(center - HUB_RADIUS))
            .size(px(HUB_RADIUS * 2.))
            .rounded_full()
            .bg(theme::gradient(
                180.,
                theme::hsla(0x1a2233, 1.),
                theme::hsla(0x080b12, 1.),
            ))
            .border_1()
            .border_color(theme::hsla(tone, if slot.is_some() { 0.62 } else { 0.24 }))
            .shadow(theme::glow(
                tone,
                if slot.is_some() { 0.26 } else { 0.1 },
                26.,
            ))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(3.))
            .child(
                div()
                    .font_family(theme::MONO_FONT)
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(if slot.is_some() { 22. } else { 10. }))
                    .line_height(px(if slot.is_some() { 24. } else { 12. }))
                    .text_color(theme::hsla(tone, if slot.is_some() { 1. } else { 0.8 }))
                    .child(caption),
            )
            .child(
                div()
                    .w(px(HUB_RADIUS * 2. - 18.))
                    .line_clamp(2)
                    .text_ellipsis()
                    .text_center()
                    .text_size(px(12.))
                    .line_height(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::TEXT))
                    .child(title),
            )
            .child(
                div()
                    .text_size(px(9.5))
                    .text_color(rgb(theme::TEXT_DIM))
                    .child(hint),
            )
    }

    fn configure_tool(&self) -> Stateful<Div> {
        let color = self.color();
        div()
            .id("wheel-configure")
            .size(px(32.))
            .rounded_full()
            .border_1()
            .border_color(theme::hsla(color, 0.24))
            .bg(theme::hsla(0x0d121c, 0.96))
            .shadow(theme::lift(0.45))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|style| {
                style
                    .bg(rgb(0x1a2232))
                    .border_color(theme::hsla(color, 0.65))
            })
            .child(theme::icon(Icon::Sliders, 14., theme::TEXT_SOFT))
    }

    fn close_tool(&self) -> Stateful<Div> {
        div()
            .id("wheel-close")
            .w(px(76.))
            .h(px(32.))
            .rounded_full()
            .border_1()
            .border_color(theme::hsla(theme::TEXT_MUTED, 0.3))
            .bg(theme::hsla(0x0d121c, 0.96))
            .shadow(theme::lift(0.45))
            .flex()
            .items_center()
            .justify_center()
            .gap(px(6.))
            .cursor_pointer()
            .hover(|style| {
                style
                    .bg(rgb(theme::DANGER_LINE))
                    .border_color(rgb(theme::DANGER))
            })
            .child(theme::icon(Icon::Close, 14., theme::TEXT))
            .child(
                div()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::TEXT))
                    .child("关闭"),
            )
    }
}

#[derive(Clone, Copy)]
struct Wedge {
    band: (f32, f32),
    span: (f32, f32),
    fill: Background,
    edge: Hsla,
}

/// 均分布局在扇区内侧画一个向外的箭头，提示“向外指向”；选中时点亮。
#[derive(Clone, Copy)]
struct Chevron {
    radius: f32,
    angle: f32,
    color: Option<u32>,
}

#[derive(Clone, Copy)]
struct Focus {
    band: (f32, f32),
    span: (f32, f32),
    color: u32,
}

/// 扇区、刻度和指针的绘制快照；半径与角度沿用浮动轮盘的逻辑几何，由 [`Stage`] 决定缩放。
struct WheelArt {
    color: u32,
    wedges: Vec<Wedge>,
    bezel: (f32, f32),
    ticks: usize,
    chevrons: Vec<Chevron>,
    focus: Option<Focus>,
}

/// 绘制舞台：`center` 为舞台中心相对 `origin` 的逻辑像素，`scale` 为相对浮动轮盘的缩放。
#[derive(Clone, Copy)]
struct Stage {
    origin: Point<Pixels>,
    center: f32,
    scale: f32,
}

impl Stage {
    /// 角度从正上方起顺时针，与选槽方向一致；半径按浮动轮盘计，返回窗口坐标。
    fn polar(self, radius: f32, angle: f32) -> Point<Pixels> {
        let radius = radius * self.scale;
        point(
            self.origin.x + px(self.center + angle.sin() * radius),
            self.origin.y + px(self.center - angle.cos() * radius),
        )
    }

    fn radii(self, radius: f32) -> Point<Pixels> {
        point(px(radius * self.scale), px(radius * self.scale))
    }

    /// 扇区缝隙按显示像素计，缩小后仍保留可见的分隔。
    fn trim(self, radius: f32) -> f32 {
        (SECTOR_GAP * self.scale).max(2.) / 2. / (radius * self.scale)
    }

    fn width(self, width: f32) -> Pixels {
        px((width * self.scale).max(1.))
    }
}

/// 轮盘配色；浮动轮盘与管理页预览共用。
pub fn accent_color(accent: WheelAccent) -> u32 {
    match accent {
        WheelAccent::Violet => 0xb5a0ff,
        WheelAccent::Amber => 0xffc36a,
        WheelAccent::Mint => 0x5df3bd,
    }
}

/// 槽位文字的中心（浮动轮盘逻辑坐标）。均分布局沿用节点位置；显式布局外圈受上下沿排除区
/// 限制而较窄，文字放在扇区径向正中。命中仍用节点几何。
fn label_center(definition: &WheelDefinition, slot: usize) -> Option<(f32, f32)> {
    let (x, y, _, _) = definition_slot_position(definition, slot)?;
    let Some(position) = definition.slots[slot].position.as_ref() else {
        return Some((x, y));
    };
    let (inner, outer) = slot_band(definition, slot)?;
    let (angle, radius) = (position_angle(position), (inner + outer) / 2.);
    let center = EXPLICIT_SIZE / 2.;
    Some((center + angle.sin() * radius, center - angle.cos() * radius))
}

/// 只描述外观，命中仍走几何函数；`ready` 判断槽位当前能否执行，不可用的选中项转为灰色。
fn wheel_art(
    definition: Option<&WheelDefinition>,
    color: u32,
    selected: Option<usize>,
    ready: impl Fn(usize) -> bool,
) -> WheelArt {
    let explicit = definition.is_some_and(explicit_layout);
    let mut art = WheelArt {
        color,
        wedges: Vec::new(),
        bezel: if explicit {
            EXPLICIT_BEZEL
        } else {
            LEGACY_BEZEL
        },
        ticks: if explicit { 120 } else { 72 },
        chevrons: Vec::new(),
        focus: None,
    };
    let Some(definition) = definition.filter(|definition| layout_valid(definition)) else {
        return art;
    };
    // 显式布局为已使用的圈补出空方位，保留稀疏布局的位置感；空方位不可执行。
    for (ring, band) in EXPLICIT_BANDS.into_iter().enumerate() {
        let positions: Vec<_> = definition
            .slots
            .iter()
            .filter_map(|slot| slot.position)
            .filter(|position| usize::from(position.ring) == ring)
            .collect();
        let Some(geometry) = positions.first().copied() else {
            continue;
        };
        for sector in 0..geometry.sectors {
            if positions.iter().any(|position| position.sector == sector) {
                continue;
            }
            let middle = position_angle(&WheelPosition { sector, ..geometry });
            let half = PI / f32::from(geometry.sectors);
            art.wedges.push(Wedge {
                band,
                span: (middle - half, middle + half),
                fill: theme::hsla(0x8fa3c4, 0.035).into(),
                edge: theme::hsla(0x8fa3c4, 0.07),
            });
        }
    }
    for slot in 0..definition.slots.len() {
        let (Some(band), Some(span)) = (slot_band(definition, slot), slot_span(definition, slot))
        else {
            continue;
        };
        let selected = selected == Some(slot);
        let ready = ready(slot);
        let tone = if ready { color } else { UNAVAILABLE };
        // CSS 角度与槽位角度同为上方起顺时针，渐变由内向外提亮。
        let outward = ((span.0 + span.1) / 2.).to_degrees();
        let (fill, edge) = if selected {
            (
                theme::gradient(outward, theme::hsla(tone, 0.07), theme::hsla(tone, 0.38)),
                theme::hsla(tone, 0.78),
            )
        } else {
            (
                theme::gradient(
                    outward,
                    theme::hsla(0x121824, 0.97),
                    theme::hsla(0x1c2434, 0.97),
                ),
                if ready {
                    theme::hsla(0x9fb2d6, 0.11)
                } else {
                    theme::hsla(theme::RECORD, 0.2)
                },
            )
        };
        art.wedges.push(Wedge {
            band,
            span,
            fill,
            edge,
        });
        if definition.slots[slot].position.is_none() {
            art.chevrons.push(Chevron {
                radius: band.0 + 15.,
                angle: (span.0 + span.1) / 2.,
                color: if selected { Some(tone) } else { None },
            });
        }
        if selected {
            art.focus = Some(Focus {
                band,
                span,
                color: tone,
            });
        }
    }
    art
}

/// 顺时针环形扇区，两侧各留半条等宽缝；跨满一圈时绘制完整圆环。
fn sector_path(
    builder: &mut PathBuilder,
    stage: Stage,
    (inner, outer): (f32, f32),
    (start, end): (f32, f32),
) {
    if end - start >= TAU - 0.001 {
        for radius in [outer, inner] {
            circle_path(builder, stage, radius);
            builder.close();
        }
        return;
    }
    let (outer_trim, inner_trim) = (stage.trim(outer), stage.trim(inner));
    let (outer_start, outer_end) = (start + outer_trim, end - outer_trim);
    let (inner_start, inner_end) = (start + inner_trim, end - inner_trim);
    if inner_end <= inner_start {
        return;
    }
    builder.move_to(stage.polar(inner, inner_start));
    builder.line_to(stage.polar(outer, outer_start));
    builder.arc_to(
        stage.radii(outer),
        px(0.),
        outer_end - outer_start > PI,
        true,
        stage.polar(outer, outer_end),
    );
    builder.line_to(stage.polar(inner, inner_end));
    builder.arc_to(
        stage.radii(inner),
        px(0.),
        inner_end - inner_start > PI,
        false,
        stage.polar(inner, inner_start),
    );
    builder.close();
}

fn circle_path(builder: &mut PathBuilder, stage: Stage, radius: f32) {
    let radii = stage.radii(radius);
    builder.move_to(stage.polar(radius, 0.));
    builder.arc_to(radii, px(0.), false, true, stage.polar(radius, PI));
    builder.arc_to(radii, px(0.), false, true, stage.polar(radius, TAU));
}

fn arc_path(builder: &mut PathBuilder, stage: Stage, radius: f32, (start, end): (f32, f32)) {
    if end - start >= TAU - 0.001 {
        circle_path(builder, stage, radius);
        return;
    }
    builder.move_to(stage.polar(radius, start));
    builder.arc_to(
        stage.radii(radius),
        px(0.),
        end - start > PI,
        true,
        stage.polar(radius, end),
    );
}

fn paint(builder: PathBuilder, color: impl Into<Background>, window: &mut Window) {
    // 极窄扇区可能无法细分；跳过这一笔，不影响其余图形与命中。
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

fn angle_within(angle: f32, (start, end): (f32, f32)) -> bool {
    let offset = (angle - (start + end) / 2. + PI).rem_euclid(TAU) - PI;
    offset.abs() <= (end - start) / 2.
}

impl WheelArt {
    fn paint(self, stage: Stage, window: &mut Window) {
        for wedge in &self.wedges {
            let mut fill = PathBuilder::fill();
            sector_path(&mut fill, stage, wedge.band, wedge.span);
            paint(fill, wedge.fill, window);
            let mut edge = PathBuilder::stroke(px(1.));
            sector_path(&mut edge, stage, wedge.band, wedge.span);
            paint(edge, wedge.edge, window);
        }

        let mut pointer = PathBuilder::stroke(px(1.));
        circle_path(&mut pointer, stage, POINTER_RADIUS);
        paint(pointer, theme::hsla(self.color, 0.18), window);

        // 外圈刻度：每 30° 一条主刻度，选中扇区范围内的刻度点亮。
        let (mut minor, mut major, mut lit) = (
            PathBuilder::stroke(px(1.)),
            PathBuilder::stroke(px(1.5)),
            PathBuilder::stroke(px(1.5)),
        );
        let major_every = (self.ticks / 12).max(1);
        for tick in 0..self.ticks {
            let angle = tick as f32 * TAU / self.ticks as f32;
            let is_major = tick % major_every == 0;
            let builder = if self
                .focus
                .is_some_and(|focus| angle_within(angle, focus.span))
            {
                &mut lit
            } else if is_major {
                &mut major
            } else {
                &mut minor
            };
            let inner = self.bezel.0 - if is_major { 2. } else { 0. };
            builder.move_to(stage.polar(inner, angle));
            builder.line_to(stage.polar(self.bezel.1, angle));
        }
        paint(minor, theme::hsla(self.color, 0.16), window);
        paint(major, theme::hsla(self.color, 0.36), window);
        if let Some(focus) = self.focus {
            paint(lit, theme::hsla(focus.color, 0.95), window);
        }

        for chevron in &self.chevrons {
            let (sin, cos) = chevron.angle.sin_cos();
            let wing = |radius: f32, side: f32| {
                let base = stage.polar(radius, chevron.angle);
                let side = side * stage.scale;
                point(base.x + px(cos * side), base.y + px(sin * side))
            };
            let mut builder = PathBuilder::stroke(stage.width(1.6));
            // 开口约 60°，斜向时仍读作箭头而不是直角括号。
            builder.move_to(wing(chevron.radius - 3.5, -4.));
            builder.line_to(wing(chevron.radius + 3.5, 0.));
            builder.line_to(wing(chevron.radius - 3.5, 4.));
            match chevron.color {
                Some(color) => paint(builder, theme::hsla(color, 1.), window),
                None => paint(builder, theme::hsla(0x9fb2d6, 0.26), window),
            }
        }

        let Some(focus) = self.focus else {
            return;
        };
        // 选中扇区外沿：宽而淡的两层光晕叠一条实线，模拟发光边。
        let trim = stage.trim(focus.band.1);
        let rim = (focus.span.0 + trim, focus.span.1 - trim);
        for (width, offset, alpha) in [(12., 2., 0.12), (6., 0.5, 0.26), (2.5, -1., 1.)] {
            let mut builder = PathBuilder::stroke(stage.width(width));
            arc_path(&mut builder, stage, focus.band.1 + offset, rim);
            paint(builder, theme::hsla(focus.color, alpha), window);
        }
        // 中心指针：在安全区外的细环上指向当前方向，弧长约 23°–50°。
        let middle = (focus.span.0 + focus.span.1) / 2.;
        let half = ((focus.span.1 - focus.span.0) / 2. - 0.05).clamp(0.2, 0.44);
        for (width, alpha) in [(8., 0.2), (2.5, 1.)] {
            let mut builder = PathBuilder::stroke(stage.width(width));
            arc_path(
                &mut builder,
                stage,
                POINTER_RADIUS,
                (middle - half, middle + half),
            );
            paint(builder, theme::hsla(focus.color, alpha), window);
        }
    }
}

/// 圆形深色底盘；底盘以外保持透明，`blur` 随舞台大小缩放。
fn disc(center: f32, radius: f32, color: u32, blur: f32) -> Div {
    div()
        .absolute()
        .left(px(center - radius))
        .top(px(center - radius))
        .size(px(radius * 2.))
        .rounded_full()
        .bg(theme::gradient(
            180.,
            theme::hsla(0x121826, 0.97),
            theme::hsla(0x06080d, 0.97),
        ))
        .border_1()
        .border_color(theme::hsla(color, 0.26))
        .shadow(vec![
            BoxShadow {
                color: theme::hsla(0x000000, 0.45),
                offset: point(px(0.), px(blur * 0.36)),
                blur_radius: px(blur),
                spread_radius: px(0.),
            },
            BoxShadow {
                color: theme::hsla(color, 0.12),
                offset: point(px(0.), px(0.)),
                blur_radius: px(blur * 1.2),
                spread_radius: px(0.),
            },
        ])
}

fn preview_scale(definition: &WheelDefinition, size: f32) -> f32 {
    let disc = if explicit_layout(definition) {
        EXPLICIT_DISC
    } else {
        LEGACY_DISC
    };
    (size / 2. - 4.) / disc
}

/// 管理页预览的槽位命中；`point` 为相对舞台左上角的逻辑像素，与扇区一致按方向和圈号判断。
pub fn preview_hit(definition: &WheelDefinition, size: f32, point: (f32, f32)) -> Option<usize> {
    let scale = preview_scale(definition, size);
    let center = wheel_size(definition) / 2.;
    hit_slot_for_definition(
        (
            center + (point.0 - size / 2.) / scale,
            center + (point.1 - size / 2.) / scale,
        ),
        definition,
    )
}

/// 管理页的缩略预览，与浮动轮盘共用扇区绘制；选中项是正在编辑的槽位，`ready` 标出无法执行的槽位。
/// `bounds` 记下舞台在窗口中的位置，供点击时换算为舞台坐标。
pub fn preview_stage(
    definition: &WheelDefinition,
    selected: Option<usize>,
    ready: impl Fn(usize) -> bool,
    size: f32,
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
) -> Div {
    let color = accent_color(definition.accent);
    let explicit = explicit_layout(definition);
    let valid = layout_valid(definition);
    let scale = preview_scale(definition, size);
    let center = size / 2.;
    let source_center = wheel_size(definition) / 2.;
    let count = if valid { definition.slots.len() } else { 0 };
    let selected = selected.filter(|&slot| slot < count);
    let states: Vec<bool> = (0..definition.slots.len()).map(ready).collect();
    let ready = |slot: usize| states.get(slot).copied().unwrap_or(false);
    let art = wheel_art(Some(definition), color, selected, ready);
    let hub_radius = HUB_RADIUS * scale;
    let tone = selected.map_or(color, |slot| if ready(slot) { color } else { UNAVAILABLE });
    let hub = div()
        .absolute()
        .left(px(center - hub_radius))
        .top(px(center - hub_radius))
        .size(px(hub_radius * 2.))
        .rounded_full()
        .bg(theme::gradient(
            180.,
            theme::hsla(0x1a2233, 1.),
            theme::hsla(0x080b12, 1.),
        ))
        .border_1()
        .border_color(theme::hsla(
            tone,
            if selected.is_some() { 0.62 } else { 0.24 },
        ))
        .shadow(theme::glow(
            tone,
            if selected.is_some() { 0.22 } else { 0.08 },
            16.,
        ))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(2.))
        .child(
            div()
                .font_family(theme::MONO_FONT)
                .font_weight(FontWeight::BOLD)
                .text_size(px(match (explicit, selected.is_some()) {
                    (false, true) => 17.,
                    (true, _) => 11.,
                    (false, false) => 9.,
                }))
                .text_color(theme::hsla(tone, if selected.is_some() { 1. } else { 0.8 }))
                .child(match selected {
                    Some(slot) => format!("{:02}", slot + 1),
                    None if explicit => format!("{count}"),
                    None => "ORBIT".into(),
                }),
        )
        .when(!explicit, |hub| {
            hub.child(
                div()
                    .w(px(hub_radius * 2. - 14.))
                    .truncate()
                    .text_center()
                    .text_size(px(10.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::TEXT))
                    .child(match selected {
                        Some(slot) => definition.slots[slot].label.clone(),
                        None => format!("{count} 个动作"),
                    }),
            )
        });
    let labels = (0..count).filter_map(|slot| {
        let (x, y) = label_center(definition, slot)?;
        let (x, y) = (
            center + (x - source_center) * scale,
            center + (y - source_center) * scale,
        );
        let (width, height) = if explicit {
            (24., 14.)
        } else {
            (NODE_WIDTH * scale, NODE_HEIGHT * scale)
        };
        let selected = selected == Some(slot);
        let ready = ready(slot);
        Some(
            div()
                .absolute()
                .left(px(x - width / 2.))
                .top(px(y - height / 2.))
                .w(px(width))
                .h(px(height))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(2.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(3.))
                        .child(
                            div()
                                .font_family(theme::MONO_FONT)
                                .text_size(px(if explicit { 8.5 } else { 9. }))
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme::hsla(
                                    if ready { color } else { theme::TEXT_DIM },
                                    if selected { 1. } else { 0.72 },
                                ))
                                .child(format!("{:02}", slot + 1)),
                        )
                        .when(!explicit && !ready, |header| {
                            header.child(theme::icon(Icon::Alert, 9., theme::RECORD))
                        }),
                )
                .when(!explicit, |label| {
                    label.child(
                        div()
                            .max_w(px(width - 4.))
                            .truncate()
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(match (ready, selected) {
                                (false, _) => theme::TEXT_DIM,
                                (true, true) => theme::TEXT,
                                (true, false) => theme::TEXT_SOFT,
                            }))
                            .child(definition.slots[slot].label.clone()),
                    )
                }),
        )
    });
    div()
        .relative()
        .flex_none()
        .size(px(size))
        .child(disc(
            center,
            if explicit { EXPLICIT_DISC } else { LEGACY_DISC } * scale,
            color,
            14.,
        ))
        .child(
            canvas(
                move |stage, _, _| bounds.set(Some(stage)),
                move |stage, (), window, _| {
                    art.paint(
                        Stage {
                            origin: stage.origin,
                            center,
                            scale,
                        },
                        window,
                    )
                },
            )
            .absolute()
            .left(px(0.))
            .top(px(0.))
            .size(px(size)),
        )
        .child(hub)
        .children(labels)
}

impl Render for RadialWheel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.drag_origin.is_none() && self.pointer_selecting && !window.is_window_hovered() {
            self.selected = None;
        }
        let count = self.count();
        let size = self
            .definition
            .as_ref()
            .map(wheel_size)
            .unwrap_or(WHEEL_SIZE);
        let center = size / 2.;
        let hovering_ready = self.drag_origin.is_none()
            && self.pointer_selecting
            && self
                .selected
                .is_some_and(|slot| self.unavailable(slot).is_none());
        let color = self.color();
        let explicit = self.definition.as_ref().is_some_and(explicit_layout);
        let chrome = chrome_at(center);
        let art = self.art();
        let status = self
            .error
            .clone()
            .or_else(|| self.selected.and_then(|slot| self.unavailable(slot)));
        let hint = status.clone().unwrap_or_else(|| {
            if self.drag_origin.is_some() {
                format!(
                    "松开鼠标键执行 · 回到起点取消（{}px 内）",
                    self.drag_cancel_radius_px
                )
            } else {
                "指向后点击 · 1–8 直达 · 方向键 + Enter · Esc / 右键取消".into()
            }
        });
        div()
            .id("radial-wheel")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .relative()
            .w(px(size))
            .h(px(size))
            .overflow_hidden()
            .font_family(theme::UI_FONT)
            .text_color(rgb(theme::TEXT))
            .when(hovering_ready, |root| root.cursor_pointer())
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::mouse_down))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if this.drag_origin.is_none()
                    && !*hovered
                    && this.pointer_selecting
                    && this.selected.take().is_some()
                {
                    cx.notify();
                }
            }))
            // 圆形底盘保证任何背景上都能读清；底盘以外保持透明。
            .child(disc(
                center,
                if explicit { EXPLICIT_DISC } else { LEGACY_DISC },
                color,
                22.,
            ))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        art.paint(
                            Stage {
                                origin: bounds.origin,
                                center,
                                scale: 1.,
                            },
                            window,
                        )
                    },
                )
                .absolute()
                .left(px(0.))
                .top(px(0.))
                .size(px(size)),
            )
            .child(self.hub(center))
            .children((0..count).map(|slot| self.slot_label(slot)))
            .child(
                self.configure_tool()
                    .absolute()
                    .left(px(chrome.configure.0))
                    .top(px(chrome.configure.1))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            if this.drag_origin.is_none() {
                                this.request(WheelRequest::Configure, window);
                            }
                            cx.notify();
                        }),
                    ),
            )
            .child(
                self.close_tool()
                    .absolute()
                    .left(px(chrome.close.0))
                    .top(px(chrome.close.1))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            this.request(WheelRequest::Close, window);
                            cx.notify();
                        }),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .left(px(24.))
                    .right(px(24.))
                    .bottom(px(8.))
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .min_h(px(26.))
                            .px(px(12.))
                            .py(px(5.))
                            .rounded(px(13.))
                            .bg(theme::hsla(0x0b0f17, 0.94))
                            .border_1()
                            .border_color(if status.is_some() {
                                theme::hsla(theme::RECORD, 0.42)
                            } else {
                                theme::hsla(0x9fb2d6, 0.12)
                            })
                            .shadow(theme::lift(0.4))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|_, _, _, cx| cx.stop_propagation()),
                            )
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .line_height(px(14.))
                                    .text_center()
                                    .line_clamp(2)
                                    .text_ellipsis()
                                    .text_color(rgb(if status.is_some() {
                                        theme::RECORD_TEXT
                                    } else {
                                        theme::TEXT_DIM
                                    }))
                                    .child(hint),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn positioned_layout(positions: &[(u8, u8, u8, bool)]) -> WheelDefinition {
        use crate::{settings::ActionSpec, wheels::WheelSlot};
        WheelDefinition {
            id: "geometry-test".into(),
            name: "几何测试".into(),
            accent: WheelAccent::Mint,
            slots: positions
                .iter()
                .enumerate()
                .map(|(index, &(ring, sector, sectors, half_step))| WheelSlot {
                    label: format!("动作{index}"),
                    action: ActionSpec::Notification,
                    position: Some(WheelPosition {
                        ring,
                        sector,
                        sectors,
                        half_step,
                    }),
                })
                .collect(),
            app_overrides: Vec::new(),
        }
    }

    #[test]
    fn floating_controls_do_not_select_or_cover_action_centers() {
        let explicit = positioned_layout(&[(0, 0, 8, false), (2, 0, 8, false)]);
        for definition in [&explicit, &crate::wheels::Wheels::default().wheels[0]] {
            let chrome = wheel_chrome(definition);
            for bounds in [chrome.configure, chrome.close] {
                assert!(bounds.0 >= 0. && bounds.1 >= 0.);
                assert!(bounds.0 + bounds.2 <= wheel_size(definition));
                assert!(bounds.1 + bounds.3 <= wheel_size(definition));
                let point = (bounds.0 + bounds.2 / 2., bounds.1 + bounds.3 / 2.);
                assert_eq!(definition_pointer_slot(point, definition), None);
            }
            for slot in 0..definition.slots.len() {
                let point = slot_position_for_definition(slot, definition).unwrap();
                assert_eq!(definition_pointer_slot(point, definition), Some(slot));
            }
        }
        // 关闭按钮下缘进入外圈的角度范围时，仍必须优先作为控件处理。
        let chrome = wheel_chrome(&explicit);
        let point = (chrome.close.0 + 0.5, chrome.close.1 + chrome.close.3 - 0.5);
        assert_eq!(hit_slot_for_definition(point, &explicit), Some(1));
        assert_eq!(definition_pointer_slot(point, &explicit), None);
    }

    #[test]
    fn drawn_sectors_stay_inside_their_selectable_range_and_below_the_controls() {
        let full: Vec<_> = (0..3)
            .flat_map(|ring| (0..16).map(move |sector| (ring, sector, 16, ring == 1)))
            .collect();
        let mut single = crate::wheels::Wheels::default().wheels[0].clone();
        single.slots.truncate(1);
        let layouts = [
            positioned_layout(&full),
            positioned_layout(&[(0, 0, 8, false), (1, 3, 16, true), (2, 4, 8, false)]),
            crate::wheels::Wheels::default().wheels[0].clone(),
            single,
        ];
        for definition in &layouts {
            let center = wheel_size(definition) / 2.;
            let chrome = wheel_chrome(definition);
            for slot in 0..definition.slots.len() {
                let (inner, outer) = slot_band(definition, slot).unwrap();
                let (start, end) = slot_span(definition, slot).unwrap();
                let middle = (start + end) / 2.;
                for radius in [inner + 0.5, (inner + outer) / 2., outer - 0.5] {
                    let point = (
                        center + middle.sin() * radius,
                        center - middle.cos() * radius,
                    );
                    assert_eq!(
                        definition_pointer_slot(point, definition),
                        Some(slot),
                        "slot {slot} radius {radius}"
                    );
                }
                for bounds in [chrome.configure, chrome.close] {
                    assert!(bounds.1 + bounds.3 < center - outer);
                }
            }
        }
        // 管理页预览按缩放后的同一扇区命中，文字中心落在自己的扇区上。
        let size = 340.;
        for definition in &layouts {
            let scale = preview_scale(definition, size);
            let center = wheel_size(definition) / 2.;
            let disc = if explicit_layout(definition) {
                EXPLICIT_DISC
            } else {
                LEGACY_DISC
            };
            assert!(disc * scale <= size / 2.);
            let stage = |(x, y): (f32, f32)| {
                (
                    size / 2. + (x - center) * scale,
                    size / 2. + (y - center) * scale,
                )
            };
            for slot in 0..definition.slots.len() {
                let (inner, outer) = slot_band(definition, slot).unwrap();
                let (start, end) = slot_span(definition, slot).unwrap();
                let middle = (start + end) / 2.;
                for radius in [inner + 1., (inner + outer) / 2., outer - 1.] {
                    let point = stage((
                        center + middle.sin() * radius,
                        center - middle.cos() * radius,
                    ));
                    assert_eq!(preview_hit(definition, size, point), Some(slot));
                }
                let label = stage(label_center(definition, slot).unwrap());
                assert_eq!(preview_hit(definition, size, label), Some(slot));
            }
            assert_eq!(preview_hit(definition, size, (size / 2., size / 2.)), None);
        }
        // 稀疏布局中间圈正上方是空方位，预览点击不选中任何槽位。
        let sparse = &layouts[1];
        let empty = (size / 2., size / 2. - 242. * preview_scale(sparse, size));
        assert_eq!(preview_hit(sparse, size, empty), None);

        assert!(angle_within(0.05, (-0.2, 0.2)));
        assert!(angle_within(TAU - 0.05, (-0.2, 0.2)));
        assert!(!angle_within(PI, (-0.2, 0.2)));
        assert!(angle_within(PI, (0., TAU)));
    }

    #[test]
    fn sparse_three_ring_centers_preserve_empty_sectors_and_half_step() {
        let definition = positioned_layout(&[(0, 0, 8, false), (1, 0, 16, true), (2, 4, 8, false)]);
        assert_eq!(wheel_size(&definition), 840.);
        for index in 0..3 {
            let center = slot_position_for_definition(index, &definition).unwrap();
            assert_eq!(hit_slot_for_definition(center, &definition), Some(index));
        }
        assert_eq!(
            slot_position_for_definition(0, &definition),
            Some((420., 298.))
        );
        let middle = slot_position_for_definition(1, &definition).unwrap();
        assert!((middle.0 - (420. + (TAU / 32.).sin() * 242.)).abs() < 0.001);
        assert_eq!(hit_slot_for_definition((542., 420.), &definition), None);
        assert_eq!(hit_slot_for_definition((419.99, 178.), &definition), None);
        assert_eq!(
            hit_slot_for_definition((420.01, 178.), &definition),
            Some(1)
        );
        assert_eq!(hit_slot_for_definition((420., 58.), &definition), None);
        assert_eq!(hit_slot_for_definition((420., 420.), &definition), None);
        assert_eq!(
            hit_slot_for_definition((f32::INFINITY, 420.), &definition),
            None
        );
        assert_eq!(hit_slot_for_definition((840., 420.), &definition), None);
        assert_eq!(hit_slot_for_definition((420., 7.), &definition), None);
        assert_eq!(slot_position_for_definition(3, &definition), None);
        let (_, _, width, height) = definition_slot_position(&definition, 1).unwrap();
        let node_corner = (middle.0 + width / 2. - 0.1, middle.1 + height / 2. - 0.1);
        assert_eq!(definition_pointer_slot(node_corner, &definition), Some(1));
    }

    #[test]
    fn explicit_drag_freezes_scale_and_ring_boundaries_and_extends_the_outer_ring() {
        let definition = positioned_layout(&[(0, 0, 8, false), (1, 0, 8, false), (2, 0, 8, false)]);
        for (radius, index) in [(181.9, 0), (182., 1), (301.9, 1), (302., 2)] {
            assert_eq!(
                hit_slot_for_definition((420., 420. - radius), &definition),
                Some(index)
            );
        }
        let origin = (1000, -1000);
        for (distance, index) in [(360, 0), (364, 1), (604, 2), (1_000_000, 2)] {
            assert_eq!(
                drag_slot_for_definition(origin, (1000, -1000 - distance), &definition, 9, 2.),
                Some(index)
            );
        }
        assert_eq!(
            drag_slot_for_definition(origin, (1000, -1009), &definition, 9, 2.),
            None
        );
        assert_eq!(
            drag_slot_for_definition(origin, (1000, -1010), &definition, 9, 100.),
            Some(0)
        );
        assert_eq!(
            drag_slot_for_definition(origin, (1000, -100_000), &definition, 9, 0.),
            None
        );
        assert_eq!(
            drag_slot_for_definition(origin, (1000, -100_000), &definition, 9, f32::NAN),
            None
        );
        assert_eq!(
            drag_slot_for_definition((0, i32::MAX), (0, i32::MIN), &definition, 9, 1.),
            Some(2)
        );
    }

    #[test]
    fn all_48_explicit_nodes_remain_visible_selectable_and_separated() {
        let positions: Vec<_> = (0..3)
            .flat_map(|ring| (0..16).map(move |sector| (ring, sector, 16, ring == 1)))
            .collect();
        let definition = positioned_layout(&positions);
        assert!(layout_valid(&definition));
        for index in 0..48 {
            let (x, y, width, height) = definition_slot_position(&definition, index).unwrap();
            assert!(x - width / 2. >= 0. && x + width / 2. <= 840.);
            assert!(y - height / 2. >= 32. && y + height / 2. <= 808.);
            assert_eq!(hit_slot_for_definition((x, y), &definition), Some(index));
            for other in index + 1..48 {
                let (other_x, other_y, other_width, other_height) =
                    definition_slot_position(&definition, other).unwrap();
                assert!(
                    (x - other_x).abs() >= (width + other_width) / 2.
                        || (y - other_y).abs() >= (height + other_height) / 2.,
                    "overlap {index}/{other}"
                );
            }
        }
        let states = vec![None; 48];
        assert_eq!(next_ready(&states, 48, Some(47), true), Some(0));
        assert_eq!(next_ready(&states, 48, Some(0), false), Some(47));
    }

    #[test]
    fn mixed_duplicate_or_inconsistent_layouts_are_not_selectable() {
        let mut definition = positioned_layout(&[(0, 0, 8, false), (0, 1, 8, false)]);
        definition.slots[1].position = None;
        assert_eq!(hit_slot_for_definition((420., 298.), &definition), None);
        definition.slots[1].position = definition.slots[0].position;
        assert!(!layout_valid(&definition));
        definition.slots[1].position = Some(WheelPosition {
            ring: 0,
            sector: 1,
            sectors: 16,
            half_step: false,
        });
        assert!(!layout_valid(&definition));
        definition.slots[1].position = Some(WheelPosition {
            ring: 0,
            sector: 1,
            sectors: 8,
            half_step: true,
        });
        assert!(!layout_valid(&definition));
        definition.slots[1].position = Some(WheelPosition {
            ring: 3,
            sector: 1,
            sectors: 8,
            half_step: false,
        });
        assert_eq!(slot_position_for_definition(0, &definition), None);
    }

    #[test]
    fn physical_drag_vectors_select_eight_directions_from_the_fixed_origin() {
        let origin = (700, -400);
        let vectors = [
            (0, -20),
            (20, -20),
            (20, 0),
            (20, 20),
            (0, 20),
            (-20, 20),
            (-20, 0),
            (-20, -20),
        ];
        for (slot, (dx, dy)) in vectors.into_iter().enumerate() {
            assert_eq!(
                drag_slot(origin, (origin.0 + dx, origin.1 + dy), 8, 9),
                Some(slot)
            );
            assert_eq!(drag_slot((0, 0), (dx, dy), 8, 9), Some(slot));
        }
    }

    #[test]
    fn drag_cancel_radius_includes_its_boundary_and_retreat_cancels_selection() {
        assert_eq!(drag_slot((0, 0), (6, 8), 8, 10), None);
        assert_eq!(drag_slot((0, 0), (7, 8), 8, 10), Some(3));
        assert_eq!(drag_slot((0, 0), (0, 0), 8, 0), None);
        assert_eq!(drag_slot((0, 0), (1, 0), 8, 0), Some(2));
        let origin = (1000, 600);
        assert_eq!(drag_slot(origin, (1030, 570), 8, 9), Some(1));
        assert_eq!(drag_slot(origin, (1009, 600), 8, 9), None);
        assert_eq!(drag_slot(origin, (1010, 600), 8, 9), Some(2));
        assert_eq!(drag_slot(origin, origin, 8, 9), None);
    }

    #[test]
    fn drag_direction_remains_selectable_far_outside_the_visible_wheel() {
        let origin = (1000, 600);
        assert_eq!(drag_slot(origin, (1_000_000, 600), 8, 9), Some(2));
        assert_eq!(drag_slot(origin, (-1_000_000, 600), 8, 9), Some(6));
        assert_eq!(drag_slot(origin, (1000, -1_000_000), 8, 9), Some(0));
        assert_eq!(drag_slot(origin, (1000, 1_000_000), 8, 9), Some(4));
        assert_eq!(drag_slot((0, 0), (99, -100), 4, 0), Some(0));
        assert_eq!(drag_slot((0, 0), (100, -100), 4, 0), Some(1));
        assert_eq!(drag_slot((0, 0), (101, -100), 4, 0), Some(1));
    }

    #[test]
    fn drag_vectors_accept_coordinate_extremes_and_reject_invalid_slot_counts() {
        let low = i32::MIN;
        let high = i32::MAX;
        assert_eq!(drag_slot((low, high), (high, low), 8, 9), Some(1));
        assert_eq!(drag_slot((high, low), (low, high), 8, 9), Some(5));
        assert_eq!(drag_slot((low, 0), (high, 0), 8, u32::MAX), None);
        assert_eq!(drag_slot((low, high), (high, low), 8, u32::MAX), Some(1));
        assert_eq!(drag_slot((low, low), (low, low), 8, 9), None);
        assert_eq!(drag_slot((0, 0), (100, 0), 0, 9), None);
        assert_eq!(drag_slot((0, 0), (100, 0), 9, 9), None);
        for position in [(0, -100), (100, 0), (0, 100), (-100, 0)] {
            assert_eq!(drag_slot((0, 0), position, 1, 9), Some(0));
        }
    }

    #[test]
    fn local_keyboard_maps_navigation_without_modifier_or_repeat_execution() {
        use omnitrigger_windows::PanelKey;
        let key = |text: &str, repeat| local_key(&Keystroke::parse(text).unwrap(), repeat);
        assert_eq!(key("1", false), Some(PanelKey::Digit(1)));
        assert_eq!(key("8", false), Some(PanelKey::Digit(8)));
        assert_eq!(key("9", false), None);
        assert_eq!(key("escape", false), Some(PanelKey::Escape));
        assert_eq!(key("enter", false), Some(PanelKey::Enter));
        assert_eq!(key("left", false), Some(PanelKey::Left));
        assert_eq!(key("right", true), Some(PanelKey::Right));
        assert_eq!(key("up", false), Some(PanelKey::Up));
        assert_eq!(key("down", false), Some(PanelKey::Down));
        assert_eq!(key("1", true), None);
        assert_eq!(key("enter", true), None);
        for modified in [
            "ctrl-1",
            "alt-1",
            "cmd-1",
            "shift-1",
            "fn-1",
            "ctrl-enter",
            "shift-left",
        ] {
            assert_eq!(key(modified, false), None, "{modified}");
        }
    }

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
