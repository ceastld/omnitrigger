//! 手势候选选择：松开时多个手势与轨迹同样相似，在指针旁列出候选，点选后执行。
//!
//! 弹窗不抢占前台；点选沿用匹配时冻结的指针位置与前台窗口，动作不会改投到后来激活的窗口。

use std::time::Duration;

use gpui::{
    div, prelude::*, px, size, Bounds, Context, FontWeight, SharedString, WeakEntity, Window,
    WindowBounds, WindowHandle, WindowKind, WindowOptions,
};
use omnitrigger_core::{Activation, GestureShape};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::gesture_editor::shape_icon;
use crate::theme::{self, hsla, rgb};
use crate::{HitSource, Workspace};

const WIDTH: f32 = 280.;
const HEADER: f32 = 34.;
const ROW: f32 = 46.;
const PADDING: f32 = 8.;
/// 无人点选时自动收起，避免弹窗长期留在屏幕上。
const TIMEOUT: Duration = Duration::from_secs(8);

fn height(rows: usize) -> f32 {
    HEADER + ROW * rows as f32 + PADDING * 2.
}

struct ChoiceItem {
    id: u64,
    title: SharedString,
    score: u16,
    shape: Option<GestureShape>,
}

pub(crate) struct GestureChooser {
    items: Vec<ChoiceItem>,
    workspace: WeakEntity<Workspace>,
}

/// 等待用户点选期间保留的匹配上下文。
pub(crate) struct PendingGestureChoice {
    window: WindowHandle<GestureChooser>,
    hwnd: isize,
    generation: u64,
    pointer: (i32, i32),
    target: Option<omnitrigger_windows::ShortcutTarget>,
}

fn raw_handle(window: &Window) -> Option<isize> {
    match HasWindowHandle::window_handle(window).map(|handle| handle.as_raw()) {
        Ok(RawWindowHandle::Win32(handle)) => Some(handle.hwnd.get()),
        _ => None,
    }
}

impl Render for GestureChooser {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div().flex().flex_col();
        for (index, item) in self.items.iter().enumerate() {
            let id = item.id;
            list = list.child(
                div()
                    .id(("gesture-choice", index))
                    .h(px(ROW))
                    .px(px(8.))
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(theme::SURFACE_HI)))
                    .when_some(item.shape, |el, shape| {
                        el.child(shape_icon(shape, 30., hsla(theme::ACCENT, 1.)))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(13.))
                            .text_color(rgb(theme::TEXT))
                            .child(item.title.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(11.5))
                            .text_color(rgb(theme::TEXT_DIM))
                            .child(format!("{}%", (item.score + 5) / 10)),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        // 先收起自己，再让工作区执行；工作区此时不再触碰本窗口。
                        window.remove_window();
                        let _ = this
                            .workspace
                            .update(cx, |workspace, cx| workspace.gesture_chosen(id, cx));
                    })),
            );
        }
        div()
            .size_full()
            .p(px(PADDING))
            .bg(rgb(theme::SURFACE))
            .border_1()
            .border_color(rgb(theme::BORDER_HI))
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(HEADER))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child("手势相近，选择要执行的一项"),
            )
            .child(list)
    }
}

impl Workspace {
    /// 多个手势与轨迹同样相似时由引擎转交：过滤掉已停用或已删除的绑定后，
    /// 只剩一条直接执行，否则在指针旁弹出候选。
    pub(crate) fn offer_gesture_choice(
        &mut self,
        candidates: Vec<(u64, u16)>,
        pointer: Option<(i32, i32)>,
        foreground: isize,
        cx: &mut Context<Self>,
    ) {
        self.close_gesture_choice(cx);
        if self.recording {
            return;
        }
        let items: Vec<ChoiceItem> = candidates
            .into_iter()
            .filter_map(|(id, score)| {
                let binding = self.binding(id).filter(|binding| binding.enabled)?;
                Some(ChoiceItem {
                    id,
                    title: binding.message.clone().into(),
                    score,
                    shape: match binding.spec.activation {
                        Activation::Gesture { shape, .. } => Some(shape),
                        _ => None,
                    },
                })
            })
            .collect();
        let target = omnitrigger_windows::capture_matching_shortcut_target(foreground).ok();
        let (pointer, first) = match (pointer, items.first()) {
            (_, None) => return,
            (Some(pointer), Some(_)) if items.len() > 1 => (pointer, None),
            // 只剩一条，或没有可用的指针位置放置弹窗：执行最相似的一条。
            (pointer, Some(first)) => (pointer.unwrap_or_default(), Some(first.id)),
        };
        if let Some(id) = first {
            self.fire_at(id, HitSource::Input, Some(pointer), target, cx);
            return;
        }
        let (width, rows) = (WIDTH, items.len());
        let workspace = cx.weak_entity();
        let mut hwnd = None;
        let bounds = Bounds::centered(None, size(px(width), px(height(rows))), cx);
        let opened = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: None,
                kind: WindowKind::PopUp,
                is_resizable: false,
                is_minimizable: false,
                is_movable: false,
                focus: false,
                // 与触发面板相同：创建回调中立即由原生层隐藏，再按指针位置显示。
                show: true,
                app_id: Some("OmniTrigger.GestureChoice".into()),
                ..Default::default()
            },
            |window, cx| {
                hwnd = raw_handle(window).filter(|&hwnd| {
                    omnitrigger_windows::prepare_gesture_choice_window(hwnd).is_ok()
                });
                cx.new(|_| GestureChooser { items, workspace })
            },
        );
        let Ok(window) = opened else {
            self.status = "无法显示手势候选。".into();
            cx.notify();
            return;
        };
        // 弹窗放在指针右下方，候选不被指针遮住。
        let center = (
            pointer.0.saturating_add((width * 0.5) as i32 + 16),
            pointer.1.saturating_add((height(rows) * 0.5) as i32 + 16),
        );
        let shown = hwnd.filter(|&hwnd| {
            omnitrigger_windows::show_gesture_choice_at_point(
                hwnd,
                center.0,
                center.1,
                width,
                height(rows),
            )
            .is_ok()
        });
        let Some(hwnd) = shown else {
            let _ = window.update(cx, |_, window, _| window.remove_window());
            self.status = "无法显示手势候选。".into();
            cx.notify();
            return;
        };
        self.gesture_choice_generation = self.gesture_choice_generation.wrapping_add(1);
        let generation = self.gesture_choice_generation;
        self.gesture_choice = Some(PendingGestureChoice {
            window,
            hwnd,
            generation,
            pointer,
            target,
        });
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TIMEOUT).await;
            let _ = this.update(cx, |this, cx| {
                if this
                    .gesture_choice
                    .as_ref()
                    .is_some_and(|pending| pending.generation == generation)
                {
                    this.close_gesture_choice(cx);
                }
            });
        })
        .detach();
    }

    /// 开发接口：`--ui-smoke` 且环境变量 `OMNITRIGGER_GESTURE_CHOICE_PREVIEW=x,y` 时，
    /// 用现有的手势绑定在该屏幕坐标预览候选弹窗。输入 Hook 不接受注入事件，
    /// 弹窗的显示只能这样在没有物理手势的情况下核验。
    pub(crate) fn preview_gesture_choice(&mut self, cx: &mut Context<Self>) {
        let Some(pointer) = std::env::var("OMNITRIGGER_GESTURE_CHOICE_PREVIEW")
            .ok()
            .and_then(|value| {
                let (x, y) = value.split_once(',')?;
                Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
            })
        else {
            return;
        };
        let candidates: Vec<(u64, u16)> = self
            .triggers
            .bindings()
            .iter()
            .filter(|binding| matches!(binding.spec.activation, Activation::Gesture { .. }))
            .take(omnitrigger_core::MAX_GESTURE_CHOICES)
            .zip([910, 880, 860, 855])
            .map(|(binding, score)| (binding.spec.id, score))
            .collect();
        self.offer_gesture_choice(candidates, Some(pointer), 0, cx);
    }

    /// 候选弹窗中点选了一项；弹窗已自行收起。
    fn gesture_chosen(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(pending) = self.gesture_choice.take() {
            self.fire_at(
                id,
                HitSource::Input,
                Some(pending.pointer),
                pending.target,
                cx,
            );
        }
    }

    /// 放弃选择并收起弹窗：超时、按 Esc、点在弹窗之外或有新的手势到来。
    pub(crate) fn close_gesture_choice(&mut self, cx: &mut Context<Self>) {
        if let Some(pending) = self.gesture_choice.take() {
            let _ = pending
                .window
                .update(cx, |_, window, _| window.remove_window());
        }
    }

    /// 物理鼠标按下在弹窗之外时收起。
    pub(crate) fn dismiss_gesture_choice_outside(
        &mut self,
        x: i32,
        y: i32,
        cx: &mut Context<Self>,
    ) {
        if self
            .gesture_choice
            .as_ref()
            .is_some_and(|pending| !omnitrigger_windows::panel_contains_point(pending.hwnd, x, y))
        {
            self.close_gesture_choice(cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_height_follows_the_number_of_candidates() {
        assert_eq!(height(2), HEADER + ROW * 2. + PADDING * 2.);
        assert!(height(4) > height(2));
    }
}
