//! 手势方向编辑：在面板中按住左键绘制，与运行时共用核心识别规则；也可逐个点选方向。

use std::{cell::Cell, rc::Rc};

use gpui::{
    canvas, div, fill, point, prelude::*, px, size, Bounds, Context, Div, Hsla, PathBuilder,
    Pixels, Point, Window,
};
use omnitrigger_core::{GestureDirection, GestureTracker, MAX_GESTURE_STEPS};

use crate::theme::{self, button, hsla, rgb, segment, small, Tone};
use crate::{caption, TriggerMode, Workspace};

/// 绘制面板只保留有限的轨迹点用于显示，识别本身不依赖这些点。
const TRAIL_LIMIT: usize = 600;
const PAD_HEIGHT: f32 = 150.;

#[derive(Default)]
pub(crate) struct GestureDraft {
    pub(crate) steps: Vec<GestureDirection>,
    tracker: Option<GestureTracker>,
    /// 相对绘制面板左上角的轨迹点；面板随布局移动或滚动时轨迹跟随。
    trail: Vec<Point<Pixels>>,
    unrecognized: bool,
    /// 面板左上角在窗口中的位置，每帧布局后更新。
    pad_origin: Rc<Cell<Point<Pixels>>>,
}

impl GestureDraft {
    pub(crate) fn load(&mut self, steps: &[GestureDirection]) {
        *self = Self {
            steps: steps.to_vec(),
            ..Self::default()
        };
    }
}

/// 识别阈值以物理像素定义；面板坐标按窗口缩放换算后再交给识别器，与运行时一致。
fn physical(position: Point<Pixels>, scale: f32) -> (i32, i32) {
    (
        (f32::from(position.x) * scale).round() as i32,
        (f32::from(position.y) * scale).round() as i32,
    )
}

/// 屏幕坐标中的单位方向向量，y 轴向下。
fn unit(direction: GestureDirection) -> (f32, f32) {
    use GestureDirection::*;
    const DIAGONAL: f32 = std::f32::consts::FRAC_1_SQRT_2;
    match direction {
        Up => (0., -1.),
        Down => (0., 1.),
        Left => (-1., 0.),
        Right => (1., 0.),
        UpLeft => (-DIAGONAL, -DIAGONAL),
        UpRight => (DIAGONAL, -DIAGONAL),
        DownLeft => (-DIAGONAL, DIAGONAL),
        DownRight => (DIAGONAL, DIAGONAL),
    }
}

fn stroke(points: &[Point<Pixels>], width: f32, color: Hsla, window: &mut Window) {
    let Some((first, rest)) = points.split_first() else {
        return;
    };
    let mut builder = PathBuilder::stroke(px(width));
    builder.move_to(*first);
    for point in rest {
        builder.line_to(*point);
    }
    // 退化的路径无法细分；跳过这一笔。
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

/// 末端箭头：以 `tip` 为尖端、沿 `direction` 方向的两条短线。
fn arrow_head(
    tip: Point<Pixels>,
    direction: (f32, f32),
    length: f32,
    width: f32,
    color: Hsla,
    window: &mut Window,
) {
    let (dx, dy) = direction;
    // 两翼相对反方向各偏转约 35°。
    let (cos, sin) = (0.819, 0.574);
    let wing = |sign: f32| {
        let (bx, by) = (-dx * cos + sign * dy * sin, -dy * cos - sign * dx * sin);
        point(tip.x + px(bx * length), tip.y + px(by * length))
    };
    stroke(&[wing(1.), tip, wing(-1.)], width, color, window);
}

/// 起点标记。
fn start_dot(center: Point<Pixels>, color: Hsla, window: &mut Window) {
    let dot = px(7.);
    window.paint_quad(
        fill(
            Bounds::new(
                point(center.x - dot / 2., center.y - dot / 2.),
                size(dot, dot),
            ),
            color,
        )
        .corner_radii(dot / 2.),
    );
}

/// 单个方向的箭头图标；八个方向同一画法，粗细与大小一致。
pub(crate) fn direction_icon(
    direction: GestureDirection,
    extent: f32,
    color: Hsla,
) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let (dx, dy) = unit(direction);
            let center = bounds.center();
            let reach = extent * 0.36;
            let width = (extent / 9.).max(1.4);
            let tail = point(center.x - px(dx * reach), center.y - px(dy * reach));
            let tip = point(center.x + px(dx * reach), center.y + px(dy * reach));
            stroke(&[tail, tip], width, color, window);
            arrow_head(tip, (dx, dy), extent * 0.34, width, color, window);
        },
    )
    .size(px(extent))
    .flex_none()
}

/// 方向序列的示意折线（单位长度）。原路折返的一笔略微偏开，避免与上一笔重叠。
fn diagram(steps: &[GestureDirection]) -> Vec<(f32, f32)> {
    let mut points = vec![(0., 0.)];
    let mut previous: Option<(f32, f32)> = None;
    for &step in steps {
        let (dx, dy) = unit(step);
        let reversed = previous.is_some_and(|(px, py)| px * dx + py * dy < -0.99);
        let (ox, oy) = if reversed {
            (-dy * 0.28, dx * 0.28)
        } else {
            (0., 0.)
        };
        let (x, y) = *points.last().expect("折线至少有起点");
        points.push((x + dx + ox, y + dy + oy));
        previous = Some((dx, dy));
    }
    points
}

/// 把示意折线缩放并居中到 `area` 内。
fn fit(points: &[(f32, f32)], area: Bounds<Pixels>) -> Vec<Point<Pixels>> {
    let (mut left, mut top, mut right, mut bottom) = (0f32, 0f32, 0f32, 0f32);
    for &(x, y) in points {
        left = left.min(x);
        top = top.min(y);
        right = right.max(x);
        bottom = bottom.max(y);
    }
    let (width, height) = (f32::from(area.size.width), f32::from(area.size.height));
    let scale = (width / (right - left).max(0.01))
        .min(height / (bottom - top).max(0.01))
        .min(54.);
    let center = area.center();
    let (mid_x, mid_y) = ((left + right) / 2., (top + bottom) / 2.);
    points
        .iter()
        .map(|&(x, y)| {
            point(
                center.x + px((x - mid_x) * scale),
                center.y + px((y - mid_y) * scale),
            )
        })
        .collect()
}

impl Workspace {
    /// 已切到手势方式但尚未给出任何方向；此时只等待绘制，不按错误提示。
    pub(crate) fn awaiting_gesture(&self) -> bool {
        self.trigger_mode == TriggerMode::Gesture && self.gesture_draft.steps.is_empty()
    }

    fn gesture_steps_changed(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        self.set_trigger_mode(TriggerMode::Gesture, cx);
    }

    fn gesture_local(&self, position: Point<Pixels>) -> Point<Pixels> {
        let origin = self.gesture_draft.pad_origin.get();
        point(position.x - origin.x, position.y - origin.y)
    }

    fn gesture_draw_start(&mut self, position: Point<Pixels>, scale: f32, cx: &mut Context<Self>) {
        let local = self.gesture_local(position);
        let draft = &mut self.gesture_draft;
        draft.tracker = Some(GestureTracker::new(physical(position, scale)));
        draft.trail.clear();
        draft.trail.push(local);
        draft.unrecognized = false;
        cx.notify();
    }

    fn gesture_draw_move(&mut self, position: Point<Pixels>, scale: f32, cx: &mut Context<Self>) {
        let local = self.gesture_local(position);
        let draft = &mut self.gesture_draft;
        let Some(tracker) = draft.tracker.as_mut() else {
            return;
        };
        tracker.push(physical(position, scale));
        let far = draft.trail.last().is_none_or(|last| {
            let (dx, dy) = (f32::from(local.x - last.x), f32::from(local.y - last.y));
            dx * dx + dy * dy >= 4.
        });
        if far && draft.trail.len() < TRAIL_LIMIT {
            draft.trail.push(local);
            cx.notify();
        }
    }

    fn gesture_draw_end(&mut self, cx: &mut Context<Self>) {
        let Some(tracker) = self.gesture_draft.tracker.take() else {
            return;
        };
        match tracker.reading().preferred() {
            Some(path) => {
                // 保留所画的原始轨迹，识别出的方向序列显示在面板底部。
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
        let drawing = draft.tracker.is_some();
        let trail = draft.trail.clone();
        let steps = draft.steps.clone();
        let origin = Rc::clone(&draft.pad_origin);
        let trail_color = hsla(
            if draft.unrecognized {
                theme::RECORD
            } else {
                theme::ACCENT
            },
            0.9,
        );
        let shape_color = hsla(theme::ACCENT, 1.);
        let start_color = hsla(theme::TEXT_SOFT, 1.);
        // 没有手绘轨迹（载入已保存的手势或点选方向）时，用规整的示意图表示方向序列。
        let show_shape = trail.is_empty() && !steps.is_empty();
        let show_sequence = !drawing && !draft.unrecognized && !steps.is_empty();
        let (hint, hint_color) = if drawing {
            ("松开左键完成", theme::TEXT_MUTED)
        } else if draft.unrecognized {
            ("没有识别出方向，请画长一些或减少转折", theme::RECORD)
        } else if steps.is_empty() {
            ("按住左键在此绘制手势", theme::TEXT_MUTED)
        } else {
            ("重新绘制可替换", theme::TEXT_FAINT)
        };
        let mut sequence = div().flex().items_center().gap(px(3.));
        for step in &steps {
            sequence = sequence.child(direction_icon(*step, 15., hsla(theme::TEXT_SOFT, 1.)));
        }
        let shape_steps = steps.clone();
        let idle = !drawing && trail.is_empty() && steps.is_empty();
        let footer = div()
            .absolute()
            .left(px(12.))
            .right(px(12.))
            .bottom(px(9.))
            .flex()
            .items_center()
            .justify_between()
            // 面板空白时提示居中；有图形或轨迹时让到底部一行。
            .when(idle, |el| el.top(px(9.)).justify_center())
            .gap(px(8.))
            .when(show_sequence, |el| el.child(sequence))
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(hint_color))
                    .child(hint),
            );
        let pad = div()
            .id("gesture-pad")
            .relative()
            .w_full()
            .h(px(PAD_HEIGHT))
            .rounded(px(10.))
            .border_1()
            .border_dashed()
            .border_color(rgb(if drawing {
                theme::ACCENT
            } else {
                theme::ACCENT_LINE
            }))
            .bg(hsla(theme::ACCENT, if drawing { 0.07 } else { 0.04 }))
            .cursor_crosshair()
            .overflow_hidden()
            .child(
                canvas(
                    move |bounds, _, _| origin.set(bounds.origin),
                    move |bounds, _, window, _| {
                        if show_shape {
                            // 底部一行留给方向序列与提示。
                            let area = Bounds::new(
                                point(bounds.origin.x + px(26.), bounds.origin.y + px(18.)),
                                size(
                                    bounds.size.width - px(52.),
                                    bounds.size.height - px(18. + 40.),
                                ),
                            );
                            let points = fit(&diagram(&shape_steps), area);
                            stroke(&points, 2.5, shape_color, window);
                            if let (Some(&first), Some(&last), Some(&step)) =
                                (points.first(), points.last(), shape_steps.last())
                            {
                                arrow_head(last, unit(step), 9., 2.5, shape_color, window);
                                start_dot(first, start_color, window);
                            }
                        } else {
                            let points: Vec<_> = trail
                                .iter()
                                .map(|position| {
                                    point(
                                        bounds.origin.x + position.x,
                                        bounds.origin.y + position.y,
                                    )
                                })
                                .collect();
                            stroke(&points, 3., trail_color, window);
                            if let (Some(&first), Some(&last)) = (points.first(), points.last()) {
                                start_dot(first, start_color, window);
                                // 画完后在末端标出走向；取稍远的一点定方向，避免末端抖动。
                                let back = points
                                    .iter()
                                    .rev()
                                    .find(|position| {
                                        let (dx, dy) = (
                                            f32::from(last.x - position.x),
                                            f32::from(last.y - position.y),
                                        );
                                        dx * dx + dy * dy >= 144.
                                    })
                                    .copied();
                                if let Some(back) = back.filter(|_| !drawing) {
                                    let (dx, dy) =
                                        (f32::from(last.x - back.x), f32::from(last.y - back.y));
                                    let length = (dx * dx + dy * dy).sqrt();
                                    arrow_head(
                                        last,
                                        (dx / length, dy / length),
                                        9.,
                                        3.,
                                        trail_color,
                                        window,
                                    );
                                }
                            }
                        }
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .child(footer)
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    this.gesture_draw_start(event.position, window.scale_factor(), cx)
                }),
            )
            .on_mouse_move(
                cx.listener(|this, event: &gpui::MouseMoveEvent, window, cx| {
                    if event.pressed_button == Some(gpui::MouseButton::Left) {
                        this.gesture_draw_move(event.position, window.scale_factor(), cx);
                    } else if this.gesture_draft.tracker.is_some() {
                        this.gesture_draw_end(cx);
                    }
                }),
            )
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
            let allowed = steps.len() < MAX_GESTURE_STEPS && steps.last() != Some(&direction);
            arrows = arrows.child(
                segment(("gesture-step", direction as usize), "", false)
                    .flex_1()
                    .min_w_0()
                    .px(px(2.))
                    .child(direction_icon(
                        direction,
                        16.,
                        hsla(theme::TEXT_MUTED, if allowed { 1. } else { 0.35 }),
                    ))
                    .when(!allowed, |el| el.cursor_default())
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
        let has_steps = !steps.is_empty();
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
                        small(button("gesture-undo", "撤销", Tone::Ghost, has_steps)).on_click(
                            cx.listener(|this, _, _, cx| {
                                if this.gesture_draft.steps.pop().is_some() {
                                    this.gesture_draft.trail.clear();
                                    this.gesture_draft.unrecognized = false;
                                    this.gesture_steps_changed(cx);
                                }
                            }),
                        ),
                    ),
            )
            .child(caption("按住鼠标键画出方向后松开触发；不移动仍是原点击。"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use GestureDirection::{Down, DownRight, Left, Right, Up};

    #[test]
    fn diagram_offsets_reversals_and_fits_inside_the_area() {
        let straight = diagram(&[Right, Down]);
        assert_eq!(straight, [(0., 0.), (1., 0.), (1., 1.)]);
        // 原路折返时终点不落在上一笔上。
        let back = diagram(&[Down, Up]);
        assert!(back[2].0.abs() > 0.2 && back[2].1.abs() < 0.01);

        let area = Bounds::new(point(px(10.), px(20.)), size(px(300.), px(90.)));
        for steps in [
            &[Right][..],
            &[Down, Up, Down, Up],
            &[Right, Down, Left, Up, Right, Down, Left, Up],
            &[DownRight, Up, DownRight],
        ] {
            for position in fit(&diagram(steps), area) {
                assert!(area.contains(&position), "{steps:?} {position:?}");
            }
        }
    }

    #[test]
    fn panel_positions_are_converted_to_physical_pixels() {
        assert_eq!(physical(point(px(10.), px(20.4)), 1.5), (15, 31));
    }
}
