mod calculator;

use gpui::{
    div, prelude::*, px, size, App, Bounds, ClipboardItem, Context, DisplayId, Div, Entity,
    Focusable, FontWeight, KeyDownEvent, Subscription, TitlebarOptions, Window, WindowBounds,
    WindowHandle, WindowKind, WindowOptions,
};

use crate::{
    appearance,
    assets::Icon,
    message_input::MessageInput,
    theme::{self, button, icon, rgb, Tone},
};
use calculator::{calculate, CalculationError, QUERY_LIMIT};
use omnitrigger_windows::ShortcutTarget;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

pub(super) fn request_foreground(
    search: WindowHandle<QuickSearch>,
    target: Option<ShortcutTarget>,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        let hwnd = search
            .update(cx, |_, window, _| {
                match HasWindowHandle::window_handle(&*window).ok()?.as_raw() {
                    RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
                    _ => None,
                }
            })
            .ok()
            .flatten();
        if let Some(hwnd) = hwnd {
            // 原生激活可能同步进入窗口回调，等 GPUI 的可变借用结束后再请求。
            if omnitrigger_windows::is_foreground_window(hwnd)
                || target.is_some_and(|target| {
                    omnitrigger_windows::ensure_shortcut_target(target).is_ok()
                })
            {
                let _ = omnitrigger_windows::activate_panel_window(hwnd);
            }
        }
    })
    .detach();
}

pub(super) struct QuickSearch {
    theme_mode: appearance::Mode,
    input: Entity<MessageInput>,
    was_active: bool,
    calculation: Option<Result<String, CalculationError>>,
    _input_changes: Subscription,
    _activation: Subscription,
}

impl QuickSearch {
    pub(super) fn open(
        display: Option<DisplayId>,
        theme_mode: appearance::Mode,
        cx: &mut App,
    ) -> Result<WindowHandle<Self>, String> {
        let bounds = Bounds::centered(display, size(px(500.), px(158.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                display_id: display,
                titlebar: Some(TitlebarOptions {
                    title: Some("OmniTrigger · 快速搜索".into()),
                    appears_transparent: true,
                    ..Default::default()
                }),
                kind: WindowKind::PopUp,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Self::new(theme_mode, window, cx)),
        )
        .map_err(|error| format!("无法打开快速搜索：{error}"))
    }

    fn new(theme_mode: appearance::Mode, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            let mut input = MessageInput::new(cx)
                .embedded()
                .with_character_limit(QUERY_LIMIT + 1);
            input.set_placeholder("输入算式，例如 (12 + 8) × 3");
            input
        });
        let input_changes = cx.observe(&input, |this, input, cx| {
            let query = input.read(cx).text();
            let next = (!query.trim().is_empty()).then(|| calculate(&query));
            if this.calculation != next {
                this.calculation = next;
                cx.notify();
            }
        });
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.was_active = true;
                window.focus(&this.input.focus_handle(cx));
                cx.notify();
            } else if this.was_active {
                window.remove_window();
            }
        });
        window.focus(&input.focus_handle(cx));
        Self {
            theme_mode,
            input,
            was_active: window.is_window_active(),
            calculation: None,
            _input_changes: input_changes,
            _activation: activation,
        }
    }

    pub(super) fn activate(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.activate_window();
        window.focus(&self.input.focus_handle(cx));
    }

    fn copy_result(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(Ok(result)) = &self.calculation {
            cx.write_to_clipboard(ClipboardItem::new_string(result.clone()));
            window.remove_window();
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.number_of_modifiers() != 0
            || self.input.read(cx).is_composing()
        {
            return;
        }
        match event.keystroke.key.as_str() {
            "enter" => self.copy_result(window, cx),
            "escape" => window.remove_window(),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn render_result(&self, cx: &mut Context<Self>) -> Div {
        let row = div().h(px(38.)).flex().items_center().gap(px(12.));
        match &self.calculation {
            Some(Ok(result)) => row
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(theme::MONO_FONT)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(22.))
                        .text_color(rgb(theme::ACCENT))
                        .child(format!("= {result}")),
                )
                .child(
                    button("copy-calculation", "复制  Enter", Tone::Primary, true)
                        .flex_none()
                        .on_click(cx.listener(|this, _, window, cx| this.copy_result(window, cx))),
                ),
            Some(Err(error)) => row
                .text_color(rgb(if *error == CalculationError::Incomplete {
                    theme::TEXT_MUTED
                } else {
                    theme::DANGER_TEXT
                }))
                .child(error.message()),
            None => row
                .text_color(rgb(theme::TEXT_MUTED))
                .child("支持括号、乘方（^ 或 **）和科学计数法"),
        }
    }
}

impl Render for QuickSearch {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        theme::apply(self.theme_mode, window.appearance());
        let focused = self.input.focus_handle(cx).is_focused(window);
        div()
            .id("quick-search")
            .size_full()
            .bg(rgb(theme::BACKGROUND))
            .border_1()
            .border_color(rgb(theme::BORDER_HI))
            .font_family(theme::UI_FONT)
            .text_color(rgb(theme::TEXT))
            .text_size(px(13.))
            .flex()
            .flex_col()
            .justify_between()
            .px(px(16.))
            .py(px(10.))
            .on_key_down(cx.listener(Self::key_down))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("快速搜索"))
                    .child(
                        div()
                            .id("close-quick-search")
                            .px(px(6.))
                            .py(px(3.))
                            .rounded(px(4.))
                            .cursor_pointer()
                            .text_color(rgb(theme::TEXT_MUTED))
                            .hover(|style| style.bg(rgb(theme::ROW)))
                            .on_click(|_, window, _| window.remove_window())
                            .child("Esc 关闭"),
                    ),
            )
            .child(
                div()
                    .h(px(44.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded(px(6.))
                    .bg(rgb(theme::ROW))
                    .border_1()
                    .border_color(rgb(if focused {
                        theme::ACCENT
                    } else {
                        theme::BORDER_HI
                    }))
                    .child(icon(Icon::Search, 16., theme::TEXT_DIM))
                    .child(div().flex_1().min_w_0().child(self.input.clone())),
            )
            .child(self.render_result(cx))
    }
}
