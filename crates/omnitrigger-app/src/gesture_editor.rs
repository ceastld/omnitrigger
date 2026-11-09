//! 手势方向编辑：在面板中按住左键绘制，与运行时共用核心识别规则；也可逐个点选方向。

use gpui::{div, fill, point, prelude::*, px, size, Bounds, Context, Div, Pixels, Point};
use omnitrigger_core::{GestureDirection, GestureTracker, MAX_GESTURE_STEPS};

use crate::theme::{self, button, hsla, rgb, segment, small, Tone};
use crate::{caption, TriggerMode, Workspace};

/// 绘制面板只保留有限的轨迹点用于显示，识别本身不依赖这些点。
const TRAIL_LIMIT: usize = 600;

#[derive(Default)]
pub(crate) struct GestureDraft {
    pub(crate) steps: Vec<GestureDirection>,
    tracker: Option<GestureTracker>,
    trail: Vec<Point<Pixels>>,
    unrecognized: bool,
}

impl GestureDraft {
    pub(crate) fn load(&mut self, steps: &[GestureDirection]) {
        *self = Self {
            steps: steps.to_vec(),
            ..Self::default()
        };
    }
}

fn pixel(position: Point<Pixels>) -> (i32, i32) {
    (
        f32::from(position.x).round() as i32,
        f32::from(position.y).round() as i32,
    )
}

impl Workspace {
    fn gesture_steps_changed(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        self.set_trigger_mode(TriggerMode::Gesture, cx);
    }

    fn gesture_draw_start(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let draft = &mut self.gesture_draft;
        draft.tracker = Some(GestureTracker::new(pixel(position)));
        draft.trail.clear();
        draft.trail.push(position);
        draft.unrecognized = false;
        cx.notify();
    }

    fn gesture_draw_move(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let draft = &mut self.gesture_draft;
        let Some(tracker) = draft.tracker.as_mut() else {
            return;
        };
        tracker.push(pixel(position));
        let far = draft.trail.last().is_none_or(|last| {
            let (dx, dy) = (
                f32::from(position.x - last.x),
                f32::from(position.y - last.y),
            );
            dx * dx + dy * dy >= 9.
        });
        if far && draft.trail.len() < TRAIL_LIMIT {
            draft.trail.push(position);
            cx.notify();
        }
    }

    fn gesture_draw_end(&mut self, cx: &mut Context<Self>) {
        let Some(tracker) = self.gesture_draft.tracker.take() else {
            return;
        };
        match tracker.reading().preferred() {
            Some(path) => {
                self.gesture_draft.steps = path.steps().to_vec();
                self.gesture_steps_changed(cx);
            }
            None => {
                self.gesture_draft.unrecognized = true;
                cx.notify();
            }
        }
    }

    pub(crate) fn gesture_fields(&self, cx: &mut Context<Self>) -> Div {
        let draft = &self.gesture_draft;
        let trail = draft.trail.clone();
        let path_text: String = draft.steps.iter().map(|step| step.arrow()).collect();
        let label = if draft.tracker.is_some() {
            "松开左键完成".to_string()
        } else if draft.unrecognized {
            "未识别出方向，请画得更长或减少转折".to_string()
        } else if path_text.is_empty() {
            "按住左键在此绘制手势".to_string()
        } else {
            path_text
        };
        let pad = div()
            .id("gesture-pad")
            .relative()
            .w_full()
            .h(px(150.))
            .rounded(px(10.))
            .border_1()
            .border_dashed()
            .border_color(rgb(theme::ACCENT_LINE))
            .bg(hsla(theme::ACCENT, 0.04))
            .cursor_crosshair()
            .flex()
            .items_center()
            .justify_center()
            .child(
                gpui::canvas(
                    |_, _, _| {},
                    move |_, _, window, _| {
                        let dot = px(4.);
                        for position in &trail {
                            window.paint_quad(fill(
                                Bounds::new(
                                    point(position.x - dot / 2., position.y - dot / 2.),
                                    size(dot, dot),
                                ),
                                hsla(theme::ACCENT, 0.7),
                            ));
                        }
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .child(
                div()
                    .text_size(if draft.steps.is_empty() || draft.tracker.is_some() {
                        px(12.5)
                    } else {
                        px(28.)
                    })
                    .text_color(rgb(if draft.unrecognized {
                        theme::RECORD
                    } else {
                        theme::TEXT_SOFT
                    }))
                    .child(label),
            )
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    this.gesture_draw_start(event.position, cx)
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                if event.pressed_button == Some(gpui::MouseButton::Left) {
                    this.gesture_draw_move(event.position, cx);
                } else if this.gesture_draft.tracker.is_some() {
                    this.gesture_draw_end(cx);
                }
            }))
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|this, _: &gpui::MouseUpEvent, _, cx| this.gesture_draw_end(cx)),
            )
            .on_mouse_up_out(
                gpui::MouseButton::Left,
                cx.listener(|this, _: &gpui::MouseUpEvent, _, cx| this.gesture_draw_end(cx)),
            );
        let mut arrows = theme::segments();
        for direction in GestureDirection::ALL {
            let allowed =
                draft.steps.len() < MAX_GESTURE_STEPS && draft.steps.last() != Some(&direction);
            arrows = arrows.child(
                segment(
                    ("gesture-step", direction as usize),
                    direction.arrow().to_string(),
                    false,
                )
                .flex_1()
                .min_w_0()
                .px(px(2.))
                .when(!allowed, |el| el.opacity(0.4))
                .on_click(cx.listener(move |this, _, _, cx| {
                    let steps = &mut this.gesture_draft.steps;
                    if steps.len() < MAX_GESTURE_STEPS && steps.last() != Some(&direction) {
                        steps.push(direction);
                        this.gesture_draft.trail.clear();
                        this.gesture_draft.unrecognized = false;
                        this.gesture_steps_changed(cx);
                    }
                })),
            );
        }
        let has_steps = !draft.steps.is_empty();
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(pad)
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .items_center()
                    .child(arrows.flex_1().min_w_0())
                    .child(
                        small(button("gesture-undo", "退格", Tone::Ghost, has_steps)).on_click(
                            cx.listener(|this, _, _, cx| {
                                if this.gesture_draft.steps.pop().is_some() {
                                    this.gesture_draft.trail.clear();
                                    this.gesture_steps_changed(cx);
                                }
                            }),
                        ),
                    ),
            )
            .child(caption(
                "按住鼠标键移动后松开时比较轨迹；未移动则保留原点击，按住不动 0.5 秒后交还原拖拽。只含上下左右的手势容许倾斜，含斜向的手势按八方向比较。",
            ))
    }
}
