//! 只呈现动作并提交请求的浮动面板；具体执行交给应用层。

use std::sync::mpsc::{SyncSender, TrySendError};

use gpui::{
    div, point, prelude::*, px, AnyElement, BoxShadow, Context, Div, Entity, FocusHandle,
    Focusable, FontWeight, KeyDownEvent, Keystroke, MouseButton, MouseDownEvent, ScrollHandle,
    SharedString, Stateful, Subscription, Window,
};

use crate::{
    assets::Icon,
    event_bridge::UiWake,
    message_input::MessageInput,
    theme::{self, button, hsla, icon, keycap, plus, rgb, small, Cap, CapSize, CapTone, Tone},
};

/// 面板固定宽度；高度随动作行数变化，见 [`panel_size`]。
pub const PANEL_WIDTH: f32 = 552.;
const COLUMNS: usize = 3;
const MAX_ROWS: usize = 4;
const HEADER_HEIGHT: f32 = 56.;
const FOOTER_HEIGHT: f32 = 46.;
const GRID_PADDING: f32 = 12.;
const TILE_HEIGHT: f32 = 100.;
const TILE_GAP: f32 = 8.;
const EMPTY_HEIGHT: f32 = 220.;
const SLOT_KEYS: usize = 9;
const SEARCH_LIMIT: usize = 64;
const SEARCH_IDLE: &str = "点击此处筛选动作";
const SEARCH_TYPING: &str = "输入以筛选动作…";

pub enum PanelRequest {
    Run(u64),
    Configure,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelKind {
    Notice,
    Command,
    Shortcut,
    Plugin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelState {
    Ready,
    Running,
    Disabled,
    /// 已启用但插件或程序当前不可用，原因放在 `status`。
    Unavailable,
}

#[derive(Clone)]
pub struct PanelItem {
    pub id: u64,
    pub title: String,
    pub caps: Vec<Cap>,
    pub kind: PanelKind,
    /// 页脚中的动作说明，例如“运行程序 · wt.exe”。
    pub detail: String,
    pub state: PanelState,
    /// 不可用原因或最近一次执行状态。
    pub status: Option<String>,
}

/// 页脚未选中动作时显示的监听状态与呼出组合。
#[derive(Clone, Default)]
pub struct PanelContext {
    pub paused: bool,
    pub shortcuts: Vec<Vec<Cap>>,
}

/// 窗口的逻辑尺寸。超过四行后在网格内滚动；筛选不改变尺寸，避免输入时窗口跳动。
pub fn panel_size(item_count: usize) -> (f32, f32) {
    let body = if item_count == 0 {
        EMPTY_HEIGHT
    } else {
        let rows = item_count.div_ceil(COLUMNS).min(MAX_ROWS) as f32;
        GRID_PADDING * 2. + rows * TILE_HEIGHT + (rows - 1.) * TILE_GAP
    };
    // 外框上下各 1px。
    (PANEL_WIDTH, HEADER_HEIGHT + body + FOOTER_HEIGHT + 2.)
}

/// 按标题、动作说明和组合文字做不区分大小写的包含匹配，返回原顺序中的下标。
pub fn filter_items(items: &[PanelItem], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            query.is_empty()
                || item.title.to_lowercase().contains(&query)
                || item.detail.to_lowercase().contains(&query)
                || item
                    .caps
                    .iter()
                    .any(|cap| cap.label.to_lowercase().contains(&query))
        })
        .map(|(index, _)| index)
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// 在三列网格中移动键盘选中项。未选中时从第一项开始；正下方没有项但还有下一行时落到最后一项。
pub fn step(selected: Option<usize>, len: usize, direction: Direction) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let Some(current) = selected.filter(|&slot| slot < len) else {
        return Some(0);
    };
    let last = len - 1;
    Some(match direction {
        Direction::Left => current.saturating_sub(1),
        Direction::Right => (current + 1).min(last),
        Direction::Up => current.checked_sub(COLUMNS).unwrap_or(current),
        Direction::Down if current + COLUMNS <= last => current + COLUMNS,
        Direction::Down if current / COLUMNS < last / COLUMNS => last,
        Direction::Down => current,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PanelKey {
    Move(Direction),
    Run,
    Escape,
    Slot(usize),
}

/// 带 Ctrl、Alt、Win 或 Fn 的按键留给系统和输入框；数字键不接受 Shift。
fn panel_key(keystroke: &Keystroke) -> Option<PanelKey> {
    let modifiers = &keystroke.modifiers;
    if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
        return None;
    }
    Some(match keystroke.key.as_str() {
        "escape" => PanelKey::Escape,
        "enter" => PanelKey::Run,
        "left" => PanelKey::Move(Direction::Left),
        "right" => PanelKey::Move(Direction::Right),
        "up" => PanelKey::Move(Direction::Up),
        "down" => PanelKey::Move(Direction::Down),
        key => match key.as_bytes() {
            [digit @ b'1'..=b'9'] if !modifiers.shift => PanelKey::Slot(usize::from(digit - b'1')),
            _ => return None,
        },
    })
}

fn first_ready(items: &[PanelItem], visible: &[usize]) -> Option<usize> {
    visible
        .iter()
        .position(|&index| items[index].state == PanelState::Ready)
}

fn ready_at(items: &[PanelItem], visible: &[usize], slot: usize) -> Option<u64> {
    let item = &items[*visible.get(slot)?];
    (item.state == PanelState::Ready).then_some(item.id)
}

/// Enter 运行键盘选中项；正在筛选且没有选中项时运行第一个可运行结果。
/// 鼠标悬停不算选中，避免呼出后误按 Enter 运行指针下的动作。
fn enter_target(
    items: &[PanelItem],
    visible: &[usize],
    selected: Option<usize>,
    query: &str,
) -> Option<u64> {
    match selected {
        Some(slot) => ready_at(items, visible, slot),
        None if !query.trim().is_empty() => ready_at(items, visible, first_ready(items, visible)?),
        None => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanelDiagnostics {
    pub present_count: u64,
    pub hide_count: u64,
    pub focus_dismiss_count: u64,
    pub last_event: &'static str,
}

impl Default for PanelDiagnostics {
    fn default() -> Self {
        Self {
            present_count: 0,
            hide_count: 0,
            focus_dismiss_count: 0,
            last_event: "尚未显示",
        }
    }
}

#[derive(Clone, Copy)]
enum HideCause {
    External,
    Outside,
    Focus,
    Escape,
    Close,
    Run,
    Configure,
}

impl HideCause {
    fn label(self) -> &'static str {
        match self {
            Self::External => "收起",
            Self::Outside => "点击外部收起",
            Self::Focus => "失焦收起",
            Self::Escape => "Esc 收起",
            Self::Close => "关闭收起",
            Self::Run => "运行后收起",
            Self::Configure => "配置后收起",
        }
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

/// hide/show 之间可能残留旧的激活通知，以原生前台身份判定。
fn is_foreground(window: &Window) -> bool {
    window_hwnd(window)
        .map(omnitrigger_windows::is_foreground_window)
        .unwrap_or_else(|| window.is_window_active())
}

pub struct TriggerPanel {
    items: Vec<PanelItem>,
    context: PanelContext,
    error: Option<String>,
    requests: SyncSender<PanelRequest>,
    wake: UiWake,
    focus: FocusHandle,
    search: Entity<MessageInput>,
    query: String,
    /// 当前筛选结果，元素为 `items` 下标。
    visible_items: Vec<usize>,
    /// 键盘选中项，元素为 `visible_items` 下标。
    selected: Option<usize>,
    hovered: Option<u64>,
    scroll: ScrollHandle,
    smooth_scroll: crate::smooth_scroll::ScrollRegions,
    visible: bool,
    was_active: bool,
    closing: bool,
    diagnostics: PanelDiagnostics,
    _activation: Subscription,
    _search: Subscription,
}

impl TriggerPanel {
    pub fn new(
        items: Vec<PanelItem>,
        context: PanelContext,
        requests: SyncSender<PanelRequest>,
        wake: UiWake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        let search = cx.new(|cx| {
            let mut input = MessageInput::new(cx)
                .with_character_limit(SEARCH_LIMIT)
                .embedded()
                .pass_arrows_when_empty();
            input.set_placeholder(SEARCH_IDLE);
            input
        });
        let search_changed = cx.observe(&search, |this, search, cx| {
            let text = search.read(cx).text();
            if this.set_query(text) {
                cx.notify();
            }
        });
        // 面板以不激活方式显示；只有点击筛选框后才会取得前台，此后失焦即收起。
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if !this.visible {
                return;
            }
            if is_foreground(window) {
                if !this.was_active {
                    this.was_active = true;
                    this.search
                        .update(cx, |input, _| input.set_placeholder(SEARCH_TYPING));
                    cx.notify();
                }
            } else if this.was_active && !this.closing {
                this.dismiss(window, HideCause::Focus);
            }
        });
        let closing_view = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            let _ = closing_view.update(cx, |view, _| view.dismiss(window, HideCause::Close));
            false
        });
        let visible_items = filter_items(&items, "");
        Self {
            items,
            context,
            error: None,
            requests,
            wake,
            focus,
            search,
            query: String::new(),
            visible_items,
            selected: None,
            hovered: None,
            scroll: ScrollHandle::new(),
            smooth_scroll: crate::smooth_scroll::ScrollRegions::default(),
            visible: false,
            was_active: false,
            closing: true,
            diagnostics: PanelDiagnostics::default(),
            _activation: activation,
            _search: search_changed,
        }
    }

    pub fn present(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.diagnostics.present_count = self.diagnostics.present_count.saturating_add(1);
        self.diagnostics.last_event = "请求显示";
        // 已在前台的可见窗口再次定位时不一定收到新的激活通知，保留失焦收起能力。
        self.was_active = self.visible && is_foreground(window);
        self.closing = false;
        self.visible = true;
        // 每次呼出都从未筛选、未选中开始。窗口不激活，筛选框的界面焦点只在点击取得前台后生效。
        let placeholder = if self.was_active {
            SEARCH_TYPING
        } else {
            SEARCH_IDLE
        };
        self.search.update(cx, |input, cx| {
            input.set_placeholder(placeholder);
            input.set_text("", cx);
        });
        self.set_query(String::new());
        self.selected = None;
        self.hovered = None;
        self.smooth_scroll.cancel("panel-items");
        self.scroll.set_offset(point(px(0.), px(0.)));
        // 未取得前台时不聚焦筛选框，避免显示光标却收不到输入；点击筛选框时再聚焦。
        if self.was_active {
            window.focus(&self.search.focus_handle(cx));
        } else {
            window.focus(&self.focus);
        }
        cx.notify();
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn diagnostics(&self) -> PanelDiagnostics {
        self.diagnostics
    }

    pub fn hide(&mut self, window: &mut Window) {
        self.hide_with_cause(window, HideCause::External);
    }

    pub fn dismiss_outside(&mut self, window: &mut Window) {
        self.dismiss(window, HideCause::Outside);
    }

    /// 输入采集未接管 Esc 时（例如监听暂停）的收起通知。
    /// 面板为筛选取得前台后由自身按键处理 Esc，先清除筛选，这里不重复收起。
    pub fn dismiss_escape(&mut self, window: &mut Window) {
        if !is_foreground(window) {
            self.dismiss(window, HideCause::Escape);
        }
    }

    fn hide_with_cause(&mut self, window: &mut Window, cause: HideCause) {
        if self.visible {
            self.diagnostics.hide_count = self.diagnostics.hide_count.saturating_add(1);
            if matches!(cause, HideCause::Focus) {
                self.diagnostics.focus_dismiss_count =
                    self.diagnostics.focus_dismiss_count.saturating_add(1);
            }
            self.diagnostics.last_event = cause.label();
        }
        self.visible = false;
        self.smooth_scroll.cancel("panel-items");
        self.closing = true;
        self.was_active = false;
        // 同步隐藏原生窗口，避免随后启动的截图拍到面板；保留窗口供下次复用。
        // 面板曾为筛选取得前台时，平台层同时把前台交还给呼出前的窗口。
        if let Some(hwnd) = window_hwnd(window) {
            omnitrigger_windows::hide_panel_window(hwnd);
        }
    }

    /// 用户点击筛选框时才请求前台，使文字输入进入面板；其余时间面板不抢占焦点。
    fn activate_for_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hwnd) = window_hwnd(window) else {
            return;
        };
        // 前台切换会同步发送窗口消息，在 GPUI 状态借用之外请求。
        cx.spawn_in(window, async move |this, cx| {
            let result = omnitrigger_windows::activate_panel_for_input(hwnd);
            let _ = this.update(cx, |this, cx| {
                let message = match result {
                    Ok(true) => return,
                    Ok(false) => "系统未授予键盘焦点，暂不能输入筛选；仍可点击动作或用方向键选择。"
                        .to_string(),
                    Err(error) => error,
                };
                this.diagnostics.last_event = "筛选未取得焦点";
                this.error = Some(message);
                cx.notify();
            });
        })
        .detach();
    }

    /// 面板未获前台时由输入采集接管的导航键，与面板取得前台后的按键共用处理。
    pub fn navigation_key(
        &mut self,
        key: omnitrigger_windows::PanelKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use omnitrigger_windows::PanelKey as Navigation;
        if !self.visible || self.closing {
            return;
        }
        let key = match key {
            Navigation::Left => PanelKey::Move(Direction::Left),
            Navigation::Right => PanelKey::Move(Direction::Right),
            Navigation::Up => PanelKey::Move(Direction::Up),
            Navigation::Down => PanelKey::Move(Direction::Down),
            Navigation::Enter => PanelKey::Run,
            Navigation::Escape => PanelKey::Escape,
            Navigation::Digit(digit @ 1..=9) => PanelKey::Slot(usize::from(digit - 1)),
            Navigation::Digit(_) => return,
        };
        self.handle_key(key, window, cx);
    }

    pub fn update_items(
        &mut self,
        items: Vec<PanelItem>,
        context: PanelContext,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if !self
            .items
            .iter()
            .map(|item| item.id)
            .eq(items.iter().map(|item| item.id))
        {
            self.smooth_scroll.cancel("panel-items");
        }
        self.items = items;
        self.context = context;
        self.error = error;
        self.visible_items = filter_items(&self.items, &self.query);
        self.selected = self
            .selected
            .filter(|&slot| slot < self.visible_items.len());
        if self
            .hovered
            .is_some_and(|id| !self.items.iter().any(|item| item.id == id))
        {
            self.hovered = None;
        }
        cx.notify();
    }

    fn set_query(&mut self, query: String) -> bool {
        if query == self.query {
            return false;
        }
        self.query = query;
        self.visible_items = filter_items(&self.items, &self.query);
        // 筛选时预选第一个可运行结果，Enter 可直接运行；清空后恢复未选中。
        self.selected = if self.query.trim().is_empty() {
            None
        } else {
            first_ready(&self.items, &self.visible_items)
        };
        self.smooth_scroll.cancel("panel-items");
        self.scroll.set_offset(point(px(0.), px(0.)));
        true
    }

    fn dismiss(&mut self, window: &mut Window, cause: HideCause) {
        if self.visible && !self.closing {
            let _ = self.requests.try_send(PanelRequest::Close);
            self.hide_with_cause(window, cause);
            self.wake.notify();
        }
    }

    fn request(&mut self, request: PanelRequest, window: &mut Window, cx: &mut Context<Self>) {
        if !self.visible || self.closing {
            return;
        }
        let cause = match request {
            PanelRequest::Run(_) => HideCause::Run,
            PanelRequest::Configure => HideCause::Configure,
            PanelRequest::Close => HideCause::Close,
        };
        match self.requests.try_send(request) {
            Ok(()) => {
                self.hide_with_cause(window, cause);
                self.wake.notify();
            }
            Err(TrySendError::Full(_)) => {
                self.diagnostics.last_event = "请求队列繁忙";
                self.error = Some("请求队列繁忙，请稍后重试。".into());
                cx.notify();
            }
            Err(TrySendError::Disconnected(_)) => {
                self.diagnostics.last_event = "请求队列断开";
                self.error = Some("触发器管理已断开，请关闭面板后重新打开。".into());
                cx.notify();
            }
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = panel_key(&event.keystroke) else {
            return;
        };
        // 已处理的按键不再翻译成字符，数字不会写入筛选框。
        cx.stop_propagation();
        self.handle_key(key, window, cx);
    }

    fn handle_key(&mut self, key: PanelKey, window: &mut Window, cx: &mut Context<Self>) {
        match key {
            PanelKey::Escape if !self.query.is_empty() => {
                self.search.update(cx, |input, cx| input.set_text("", cx));
            }
            PanelKey::Escape => self.dismiss(window, HideCause::Escape),
            PanelKey::Move(direction) => {
                self.selected = step(self.selected, self.visible_items.len(), direction);
                if let Some(slot) = self.selected {
                    self.smooth_scroll.cancel("panel-items");
                    self.scroll.scroll_to_item(slot);
                }
                cx.notify();
            }
            PanelKey::Run => {
                if let Some(id) =
                    enter_target(&self.items, &self.visible_items, self.selected, &self.query)
                {
                    self.request(PanelRequest::Run(id), window, cx);
                }
            }
            PanelKey::Slot(slot) => {
                if let Some(id) = ready_at(&self.items, &self.visible_items, slot) {
                    self.request(PanelRequest::Run(id), window, cx);
                }
            }
        }
    }

    /// 页脚说明的对象：键盘选中优先，其次是鼠标悬停的动作。
    /// 指针直接离开窗口时元素不会收到悬停结束，以窗口悬停状态兜底。
    fn active_item(&self, window_hovered: bool) -> Option<&PanelItem> {
        self.selected
            .and_then(|slot| self.visible_items.get(slot))
            .map(|&index| &self.items[index])
            .or_else(|| {
                let id = self.hovered.filter(|_| window_hovered)?;
                self.visible_items
                    .iter()
                    .map(|&index| &self.items[index])
                    .find(|item| item.id == id)
            })
    }

    fn header(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        // 不激活的窗口不会收到键盘输入，只在面板取得前台后显示输入框焦点。
        let focused = self.was_active && self.search.focus_handle(cx).is_focused(window);
        div()
            .flex_none()
            .h(px(HEADER_HEIGHT))
            .pl(px(14.))
            .pr(px(10.))
            .border_b_1()
            .border_color(rgb(theme::DIVIDER))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(theme::logo(26.).mr(px(2.)))
            .child(
                div()
                    .id("panel-search")
                    .flex_1()
                    .min_w_0()
                    .h(px(34.))
                    .px(px(10.))
                    .rounded(px(8.))
                    .bg(rgb(theme::SURFACE))
                    .border_1()
                    .border_color(rgb(if focused {
                        theme::ACCENT_LINE
                    } else {
                        theme::BORDER_MID
                    }))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, window, cx| {
                            window.focus(&this.search.focus_handle(cx));
                            if !this.was_active {
                                this.activate_for_input(window, cx);
                            }
                        }),
                    )
                    .child(icon(Icon::Search, 14., theme::TEXT_FAINT))
                    .child(div().flex_1().min_w_0().child(self.search.clone())),
            )
            .child(status_pill(self.context.paused))
            .child(
                tool_button("panel-configure", Icon::Sliders, 16.)
                    .ml(px(2.))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.request(PanelRequest::Configure, window, cx);
                    })),
            )
            .child(
                tool_button("panel-close", Icon::Close, 15.).on_click(
                    cx.listener(|this, _, window, _| this.dismiss(window, HideCause::Close)),
                ),
            )
    }

    fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.items.is_empty() {
            return div()
                .flex_1()
                .min_h_0()
                .p(px(24.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(10.))
                .child(
                    div()
                        .size(px(44.))
                        .rounded(px(12.))
                        .bg(rgb(theme::SURFACE))
                        .border_1()
                        .border_color(rgb(theme::BORDER_MID))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon(Icon::Keyboard, 22., theme::TEXT_DIM)),
                )
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::TEXT_SOFT))
                        .child("还没有可运行的动作"),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(theme::TEXT_DIM))
                        .child("前往管理触发器，录制组合并配置动作。"),
                )
                .child(
                    small(button(
                        "panel-empty-configure",
                        "管理触发器",
                        Tone::Secondary,
                        true,
                    ))
                    .mt(px(6.))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.request(PanelRequest::Configure, window, cx);
                    })),
                )
                .into_any_element();
        }
        if self.visible_items.is_empty() {
            return div()
                .flex_1()
                .min_h_0()
                .p(px(24.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::TEXT_SOFT))
                        .child(format!("没有匹配“{}”的动作", self.query.trim())),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(theme::TEXT_DIM))
                        .child("按 Esc 清除筛选"),
                )
                .into_any_element();
        }
        div()
            .id("panel-items")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if !*hovered && this.hovered.take().is_some() {
                    cx.notify();
                }
            }))
            .p(px(GRID_PADDING))
            .grid()
            .grid_cols(COLUMNS as u16)
            .gap(px(TILE_GAP))
            .content_start()
            .children(
                self.visible_items
                    .iter()
                    .enumerate()
                    .map(|(slot, &index)| self.tile(slot, &self.items[index], cx)),
            )
            .map(|el| {
                self.smooth_scroll
                    .wrap_with_handle("panel-items", el, &self.scroll)
            })
            .into_any_element()
    }

    fn tile(&self, slot: usize, item: &PanelItem, cx: &mut Context<Self>) -> Stateful<Div> {
        let id = item.id;
        let ready = item.state == PanelState::Ready;
        let selected = self.selected == Some(slot);
        let lit = selected && ready;
        let group = SharedString::from(format!("panel-tile-{id}"));
        let glyph = match item.kind {
            PanelKind::Notice => Icon::Bell,
            PanelKind::Command => Icon::Terminal,
            PanelKind::Shortcut => Icon::Keyboard,
            PanelKind::Plugin => Icon::Capture,
        };
        let glyph_color = match item.state {
            _ if lit => theme::ACCENT,
            PanelState::Running => theme::RECORD,
            PanelState::Unavailable => theme::TEXT_DIM,
            _ => theme::ICON,
        };
        let badge = div()
            .flex_none()
            .size(px(30.))
            .rounded(px(8.))
            .bg(rgb(if lit {
                theme::ACCENT_DEEP
            } else {
                theme::SURFACE_HI
            }))
            .border_1()
            .border_color(rgb(if lit {
                theme::ACCENT_LINE
            } else {
                theme::BORDER_MID
            }))
            .flex()
            .items_center()
            .justify_center()
            .when(ready, |el| {
                el.group_hover(group.clone(), |style| {
                    style
                        .bg(rgb(theme::ACCENT_DEEP))
                        .border_color(rgb(theme::ACCENT_LINE))
                })
            })
            .child(icon(glyph, 16., glyph_color).when(ready, |el| {
                el.group_hover(group.clone(), |style| style.text_color(rgb(theme::ACCENT)))
            }));
        let corner = match item.state {
            PanelState::Ready if slot < SLOT_KEYS => {
                slot_key(slot + 1, lit, group.clone()).into_any_element()
            }
            PanelState::Ready => div().into_any_element(),
            PanelState::Running => div()
                .flex()
                .items_center()
                .gap(px(5.))
                .text_size(px(11.))
                .text_color(rgb(theme::RECORD))
                .whitespace_nowrap()
                .child(theme::dot(theme::RECORD, 6.))
                .child("执行中")
                .into_any_element(),
            PanelState::Disabled => div()
                .text_size(px(11.))
                .text_color(rgb(theme::TEXT_DIM))
                .whitespace_nowrap()
                .child("已停用")
                .into_any_element(),
            PanelState::Unavailable => div()
                .flex()
                .items_center()
                .gap(px(4.))
                .text_size(px(11.))
                .text_color(rgb(theme::RECORD))
                .whitespace_nowrap()
                .child(icon(Icon::Alert, 12., theme::RECORD))
                .child("未就绪")
                .into_any_element(),
        };
        div()
            .id(("panel-action", id))
            .group(group)
            .h(px(TILE_HEIGHT))
            .min_w_0()
            .p(px(12.))
            .rounded(px(10.))
            .bg(rgb(if lit {
                theme::SURFACE_HI
            } else {
                theme::SURFACE
            }))
            .border_1()
            .border_color(rgb(if lit {
                theme::ACCENT
            } else if selected {
                theme::BORDER_MID
            } else {
                theme::BORDER
            }))
            .when(lit, |el| el.shadow(vec![focus_ring()]))
            .when(item.state == PanelState::Disabled, |el| el.opacity(0.45))
            .flex()
            .flex_col()
            .gap(px(8.))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                // 鼠标接管说明对象时清除键盘选中，避免页脚与高亮指向不同动作。
                if *hovered && this.hovered != Some(id) {
                    this.hovered = Some(id);
                    this.selected = None;
                    cx.notify();
                }
            }))
            .when(ready, |el| {
                el.cursor_pointer()
                    .hover(|style| {
                        style
                            .bg(rgb(theme::SURFACE_HI))
                            .border_color(rgb(theme::ACCENT_LINE))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.request(PanelRequest::Run(id), window, cx);
                    }))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .child(badge)
                    .child(corner),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .line_height(px(19.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(if item.state == PanelState::Unavailable {
                        theme::TEXT_SOFT
                    } else {
                        theme::TEXT
                    }))
                    .line_clamp(2)
                    .text_ellipsis()
                    .child(item.title.clone()),
            )
    }

    fn footer(&self, window_hovered: bool) -> Div {
        let base = div()
            .flex_none()
            .h(px(FOOTER_HEIGHT))
            .px(px(14.))
            .border_t_1()
            .border_color(rgb(theme::DIVIDER))
            .bg(rgb(theme::SIDEBAR))
            .flex()
            .items_center()
            .gap(px(12.))
            .text_size(px(11.5))
            .text_color(rgb(theme::TEXT_DIM))
            .whitespace_nowrap();
        if let Some(item) = self.active_item(window_hovered) {
            let ready = item.state == PanelState::Ready;
            let note = match item.state {
                PanelState::Ready => item.status.clone().map(|text| (text, theme::TEXT_DIM)),
                PanelState::Running => Some(("正在执行，完成前不能再次运行".into(), theme::RECORD)),
                PanelState::Disabled => {
                    Some(("已停用，可在管理触发器中启用".into(), theme::TEXT_DIM))
                }
                PanelState::Unavailable => Some((
                    item.status.clone().unwrap_or_else(|| "当前不可用".into()),
                    theme::RECORD,
                )),
            };
            return base
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(caps_row(&item.caps))
                        .child(
                            div()
                                .flex_none()
                                .w(px(1.))
                                .h(px(14.))
                                .bg(rgb(theme::BORDER_MID)),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_color(rgb(theme::TEXT_MUTED))
                                .child(item.detail.clone()),
                        )
                        .when_some(note, |el, (text, color)| {
                            el.child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(rgb(color))
                                    .child(text),
                            )
                        }),
                )
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        // Enter 只运行键盘选中项；悬停时提示点击。
                        .when(ready && self.selected.is_some(), |el| {
                            el.child(hint_icon(Icon::Enter))
                                .child(div().mr(px(6.)).child("运行"))
                        })
                        .when(ready && self.selected.is_none(), |el| {
                            el.child(div().mr(px(6.)).child("点击运行"))
                        })
                        .child(hint_key("Esc"))
                        .child("关闭"),
                );
        }
        let left = if self.context.paused {
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(theme::dot(theme::RECORD, 6.))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(rgb(theme::RECORD_TEXT))
                        .child("监听已暂停 · 组合不会触发，仍可点击动作"),
                )
        } else if self.context.shortcuts.is_empty() {
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(rgb(theme::TEXT_FAINT))
                .child("尚未绑定呼出组合，可在主窗口录制")
        } else {
            let mut row = div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    div()
                        .flex_none()
                        .mr(px(2.))
                        .text_color(rgb(theme::TEXT_FAINT))
                        .child("呼出"),
                );
            for (index, caps) in self.context.shortcuts.iter().take(2).enumerate() {
                if index > 0 {
                    row = row.child(
                        div()
                            .flex_none()
                            .mx(px(2.))
                            .text_color(rgb(theme::TEXT_FAINT))
                            .child("或"),
                    );
                }
                row = row.child(caps_row(caps));
            }
            row
        };
        let right = div().flex_none().flex().items_center().gap(px(6.));
        let right = if !self.query.is_empty() {
            right.child(hint_key("Esc")).child("清除筛选")
        } else if self.context.paused && !self.was_active {
            // 暂停时输入采集不接管导航键，需要先点击筛选框让面板取得前台。
            right.child("点击筛选框后可用键盘")
        } else {
            right
                .child(hint_key("↑↓←→"))
                .child(div().mr(px(6.)).child("选择"))
                .child(hint_key("1–9"))
                .child("运行")
        };
        base.child(left).child(right)
    }
}

fn focus_ring() -> BoxShadow {
    BoxShadow {
        color: hsla(theme::ACCENT, 1.),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(1.),
    }
}

fn status_pill(paused: bool) -> Div {
    let (bg, line, dot, text, label) = if paused {
        (
            theme::RECORD_BG,
            theme::RECORD_LINE,
            theme::RECORD,
            theme::RECORD_TEXT,
            "已暂停",
        )
    } else {
        (
            theme::ACCENT_BG,
            theme::ACCENT_LINE,
            theme::ACCENT,
            theme::TEXT_MUTED,
            "监听中",
        )
    };
    div()
        .flex_none()
        .h(px(24.))
        .px(px(9.))
        .rounded(px(12.))
        .bg(rgb(bg))
        .border_1()
        .border_color(rgb(line))
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(11.5))
        .text_color(rgb(text))
        .whitespace_nowrap()
        .child(theme::dot(dot, 6.))
        .child(label)
}

fn tool_button(id: &'static str, glyph: Icon, size: f32) -> Stateful<Div> {
    div()
        .id(id)
        .group(id)
        .flex_none()
        .size(px(32.))
        .rounded(px(8.))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|style| style.bg(rgb(theme::SURFACE_HI)))
        .child(
            icon(glyph, size, theme::TEXT_MUTED)
                .group_hover(id, |style| style.text_color(rgb(theme::TEXT))),
        )
}

/// 卡片右上角的数字键，悬停或选中时提亮。
fn slot_key(number: usize, lit: bool, group: SharedString) -> Div {
    div()
        .flex_none()
        .rounded(px(4.))
        .bg(rgb(theme::KEYCAP_BASE))
        .pb(px(1.5))
        .child(
            div()
                .min_w(px(18.))
                .h(px(17.))
                .px(px(4.))
                .rounded(px(4.))
                .bg(rgb(if lit { theme::KEYCAP } else { theme::SELECTED }))
                .border_1()
                .border_color(rgb(if lit {
                    theme::KEYCAP_EDGE
                } else {
                    theme::BORDER_MID
                }))
                .group_hover(group, |style| {
                    style
                        .bg(rgb(theme::KEYCAP))
                        .border_color(rgb(theme::KEYCAP_EDGE))
                        .text_color(rgb(theme::TEXT))
                })
                .flex()
                .items_center()
                .justify_center()
                .font_family(theme::MONO_FONT)
                .text_size(px(10.5))
                .text_color(rgb(if lit { theme::TEXT } else { theme::TEXT_DIM }))
                .child(number.to_string()),
        )
}

fn hint_shell(content: impl IntoElement) -> Div {
    div()
        .flex_none()
        .rounded(px(4.))
        .bg(rgb(theme::KEYCAP_BASE))
        .pb(px(1.5))
        .child(
            div()
                .h(px(18.))
                .px(px(5.))
                .rounded(px(4.))
                .bg(rgb(theme::KEYCAP))
                .border_1()
                .border_color(rgb(theme::KEYCAP_EDGE))
                .flex()
                .items_center()
                .font_family(theme::MONO_FONT)
                .text_size(px(10.))
                .text_color(rgb(theme::TEXT_SOFT))
                .child(content),
        )
}

fn hint_key(label: &'static str) -> Div {
    hint_shell(label)
}

fn hint_icon(glyph: Icon) -> Div {
    hint_shell(icon(glyph, 11., theme::TEXT_SOFT))
}

/// 单行组合键帽；页脚高度固定，不换行。
fn caps_row(caps: &[Cap]) -> Div {
    let mut row = div().flex_none().flex().items_center().gap(px(5.));
    for (index, cap) in caps.iter().enumerate() {
        if index > 0 {
            row = row.child(plus(CapSize::Small, theme::TEXT_FAINT));
        }
        row = row.child(keycap(cap, CapSize::Small, CapTone::Normal));
    }
    row
}

impl Render for TriggerPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("trigger-panel")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .size_full()
            .font_family(theme::UI_FONT)
            .text_color(rgb(theme::TEXT))
            .bg(rgb(theme::BACKGROUND))
            .border_1()
            .border_color(rgb(theme::BORDER_HI))
            .rounded(px(14.))
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(self.header(window, cx))
            .when_some(self.error.clone(), |el, error| {
                el.child(
                    div()
                        .flex_none()
                        .min_h(px(40.))
                        .px(px(14.))
                        .py(px(10.))
                        .bg(rgb(theme::DANGER_BG))
                        .border_b_1()
                        .border_color(rgb(theme::DANGER_LINE))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_size(px(12.))
                        .text_color(rgb(theme::DANGER_TEXT))
                        .child(icon(Icon::Alert, 14., theme::DANGER))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .line_clamp(2)
                                .text_ellipsis()
                                .child(error),
                        ),
                )
            })
            .child(self.body(cx))
            .child(self.footer(window.is_window_hovered()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: u64, title: &str, detail: &str, state: PanelState) -> PanelItem {
        PanelItem {
            id,
            title: title.into(),
            caps: vec![Cap {
                label: "CapsLock".into(),
                mouse: false,
                arrow: None,
            }],
            kind: PanelKind::Command,
            detail: detail.into(),
            state,
            status: None,
        }
    }

    #[test]
    fn panel_height_follows_rows_and_stops_growing_after_four() {
        assert_eq!(panel_size(0), (552., 324.));
        assert_eq!(panel_size(1), (552., 228.));
        assert_eq!(panel_size(3), (552., 228.));
        assert_eq!(panel_size(4), (552., 336.));
        assert_eq!(panel_size(12), (552., 552.));
        assert_eq!(panel_size(40), panel_size(12));
    }

    #[test]
    fn filter_matches_title_detail_and_caps_without_case_or_padding() {
        let items = vec![
            item(1, "截图", "RaiShot · 插件动作", PanelState::Ready),
            item(
                2,
                "打开 Windows 终端",
                "运行程序 · wt.exe",
                PanelState::Ready,
            ),
            item(3, "番茄钟提醒", "仅提示", PanelState::Disabled),
        ];
        assert_eq!(filter_items(&items, ""), vec![0, 1, 2]);
        assert_eq!(filter_items(&items, "  WT.EXE "), vec![1]);
        assert_eq!(filter_items(&items, "raishot"), vec![0]);
        assert_eq!(filter_items(&items, "截图"), vec![0]);
        assert_eq!(filter_items(&items, "capslock"), vec![0, 1, 2]);
        assert!(filter_items(&items, "备份").is_empty());
    }

    #[test]
    fn grid_steps_stay_inside_three_columns_and_land_on_the_last_row() {
        assert_eq!(step(None, 0, Direction::Down), None);
        assert_eq!(step(None, 8, Direction::Left), Some(0));
        assert_eq!(step(Some(9), 8, Direction::Right), Some(0));
        assert_eq!(step(Some(0), 8, Direction::Left), Some(0));
        assert_eq!(step(Some(0), 8, Direction::Right), Some(1));
        assert_eq!(step(Some(7), 8, Direction::Right), Some(7));
        assert_eq!(step(Some(1), 8, Direction::Down), Some(4));
        assert_eq!(step(Some(4), 8, Direction::Down), Some(7));
        assert_eq!(step(Some(5), 8, Direction::Down), Some(7));
        assert_eq!(step(Some(7), 8, Direction::Down), Some(7));
        assert_eq!(step(Some(4), 8, Direction::Up), Some(1));
        assert_eq!(step(Some(2), 8, Direction::Up), Some(2));
        assert_eq!(step(Some(1), 3, Direction::Down), Some(1));
    }

    #[test]
    fn keys_map_without_modifiers_and_digits_reject_shift() {
        let key = |text: &str| panel_key(&Keystroke::parse(text).unwrap());
        assert_eq!(key("escape"), Some(PanelKey::Escape));
        assert_eq!(key("enter"), Some(PanelKey::Run));
        assert_eq!(key("shift-enter"), Some(PanelKey::Run));
        assert_eq!(key("up"), Some(PanelKey::Move(Direction::Up)));
        assert_eq!(key("1"), Some(PanelKey::Slot(0)));
        assert_eq!(key("9"), Some(PanelKey::Slot(8)));
        assert_eq!(key("0"), None);
        assert_eq!(key("shift-1"), None);
        assert_eq!(key("ctrl-1"), None);
        assert_eq!(key("alt-left"), None);
        assert_eq!(key("a"), None);
    }

    #[test]
    fn enter_and_digits_only_run_ready_items_in_the_filtered_order() {
        let items = vec![
            item(10, "同步笔记仓库", "运行程序", PanelState::Running),
            item(11, "截图", "插件", PanelState::Unavailable),
            item(12, "打开下载目录", "运行程序", PanelState::Ready),
            item(13, "导出日志", "运行程序", PanelState::Disabled),
        ];
        let all = filter_items(&items, "");
        assert_eq!(first_ready(&items, &all), Some(2));
        assert_eq!(enter_target(&items, &all, None, ""), None);
        assert_eq!(enter_target(&items, &all, Some(0), ""), None);
        assert_eq!(enter_target(&items, &all, Some(2), ""), Some(12));
        assert_eq!(enter_target(&items, &all, None, " 运行 "), Some(12));
        assert_eq!(enter_target(&items, &all, Some(0), "运行"), None);
        assert_eq!(ready_at(&items, &all, 3), None);
        assert_eq!(ready_at(&items, &all, 8), None);
        let filtered = filter_items(&items, "下载");
        assert_eq!(ready_at(&items, &filtered, 0), Some(12));
        assert_eq!(first_ready(&items, &filter_items(&items, "日志")), None);
    }
}
