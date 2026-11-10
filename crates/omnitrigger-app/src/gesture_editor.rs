//! 手势形状编辑：在面板中按住左键绘制，所画轨迹经与运行时相同的规则化为形状模板。

use std::{cell::Cell, rc::Rc};

use gpui::{
    canvas, div, fill, point, prelude::*, px, size, Bounds, Context, Div, Hsla, PathBuilder,
    Pixels, Point, Window,
};
use omnitrigger_core::{GestureShape, GestureTrace};

use crate::theme::{self, hsla, rgb};
use crate::{caption, TriggerMode, Workspace};

/// 绘制面板只保留有限的轨迹点用于显示，形状本身不依赖这些点。
const TRAIL_LIMIT: usize = 600;
const PAD_HEIGHT: f32 = 150.;
/// 面板中显示已保存形状时较长一边的长度。
const SHAPE_EXTENT: f32 = 96.;

#[derive(Default)]
pub(crate) struct GestureDraft {
    pub(crate) shape: Option<GestureShape>,
    trace: Option<GestureTrace>,
    /// 相对绘制面板左上角的轨迹点；面板随布局移动或滚动时轨迹跟随。
    trail: Vec<Point<Pixels>>,
    too_short: bool,
    /// 面板左上角在窗口中的位置，每帧布局后更新。
    pad_origin: Rc<Cell<Point<Pixels>>>,
}

impl GestureDraft {
    pub(crate) fn load(&mut self, shape: Option<GestureShape>) {
        *self = Self {
            shape,
            ..Self::default()
        };
    }
}

/// 轨迹长度下限以物理像素定义；面板坐标按窗口缩放换算后再记录，与运行时一致。
fn physical(position: Point<Pixels>, scale: f32) -> (i32, i32) {
    (
        (f32::from(position.x) * scale).round() as i32,
        (f32::from(position.y) * scale).round() as i32,
    )
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

fn start_dot(center: Point<Pixels>, diameter: f32, color: Hsla, window: &mut Window) {
    let dot = px(diameter);
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

/// 画一条带起点与末端箭头的轨迹；末端走向取稍远的一点，避免被末端抖动带偏。
fn paint_route(
    points: &[Point<Pixels>],
    width: f32,
    color: Hsla,
    start_color: Hsla,
    arrow: bool,
    window: &mut Window,
) {
    stroke(points, width, color, window);
    let (Some(&first), Some(&last)) = (points.first(), points.last()) else {
        return;
    };
    start_dot(first, width * 2.4, start_color, window);
    if !arrow {
        return;
    }
    let reach = (width * 4.).max(6.);
    let back = points.iter().rev().find(|position| {
        let (dx, dy) = (
            f32::from(last.x - position.x),
            f32::from(last.y - position.y),
        );
        dx * dx + dy * dy >= reach * reach
    });
    if let Some(back) = back {
        let (dx, dy) = (f32::from(last.x - back.x), f32::from(last.y - back.y));
        let length = (dx * dx + dy * dy).sqrt();
        arrow_head(
            last,
            (dx / length, dy / length),
            width * 3.2,
            width,
            color,
            window,
        );
    }
}

/// 把归一化形状放到以 `center` 为中心、较长一边为 `extent` 的范围内。
fn place(shape: &GestureShape, center: Point<Pixels>, extent: f32) -> Vec<Point<Pixels>> {
    let scale = extent / 2000.;
    shape
        .points()
        .iter()
        .map(|&(x, y)| {
            point(
                center.x + px(f32::from(x) * scale),
                center.y + px(f32::from(y) * scale),
            )
        })
        .collect()
}

/// 手势形状的小图标：带起点与末端箭头的轨迹缩略图。
pub(crate) fn shape_icon(shape: GestureShape, extent: f32, color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let width = (extent / 11.).max(1.4);
            // 四周留出线宽与箭头的余量。
            let points = place(&shape, bounds.center(), extent - width * 5.);
            paint_route(&points, width, color, color, true, window);
        },
    )
    .size(px(extent))
    .flex_none()
}

impl Workspace {
    /// 已切到手势方式但尚未画出形状；此时只等待绘制，不按错误提示。
    pub(crate) fn awaiting_gesture(&self) -> bool {
        self.trigger_mode == TriggerMode::Gesture && self.gesture_draft.shape.is_none()
    }

    fn gesture_local(&self, position: Point<Pixels>) -> Point<Pixels> {
        let origin = self.gesture_draft.pad_origin.get();
        point(position.x - origin.x, position.y - origin.y)
    }

    fn gesture_draw_start(&mut self, position: Point<Pixels>, scale: f32, cx: &mut Context<Self>) {
        let local = self.gesture_local(position);
        let draft = &mut self.gesture_draft;
        draft.trace = Some(GestureTrace::new(physical(position, scale)));
        draft.trail.clear();
        draft.trail.push(local);
        draft.too_short = false;
        cx.notify();
    }

    fn gesture_draw_move(&mut self, position: Point<Pixels>, scale: f32, cx: &mut Context<Self>) {
        let local = self.gesture_local(position);
        let draft = &mut self.gesture_draft;
        let Some(trace) = draft.trace.as_mut() else {
            return;
        };
        trace.push(physical(position, scale));
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
        let Some(trace) = self.gesture_draft.trace.take() else {
            return;
        };
        match trace.shape() {
            Ok(shape) => {
                // 所画轨迹就是手势本身：保留原样显示，保存的是它的归一化形状。
                self.gesture_draft.shape = Some(shape);
                self.error = None;
                self.set_trigger_mode(TriggerMode::Gesture, cx);
            }
            Err(_) => {
                self.gesture_draft.too_short = true;
                cx.notify();
            }
        }
    }

    pub(crate) fn gesture_fields(&self, cx: &mut Context<Self>) -> Div {
        let draft = &self.gesture_draft;
        let drawing = draft.trace.is_some();
        let trail = draft.trail.clone();
        let shape = draft.shape;
        let origin = Rc::clone(&draft.pad_origin);
        let color = hsla(
            if draft.too_short {
                theme::RECORD
            } else {
                theme::ACCENT
            },
            if draft.too_short { 0.9 } else { 1. },
        );
        let start_color = hsla(theme::TEXT_SOFT, 1.);
        let idle = !drawing && trail.is_empty() && shape.is_none();
        let (hint, hint_color) = if drawing {
            ("松开左键完成", theme::TEXT_MUTED)
        } else if draft.too_short {
            ("太短了，请画长一些", theme::RECORD)
        } else if shape.is_none() {
            ("按住左键在此绘制手势", theme::TEXT_MUTED)
        } else {
            ("重新绘制可替换", theme::TEXT_FAINT)
        };
        let footer = div()
            .absolute()
            .left(px(12.))
            .right(px(12.))
            .bottom(px(9.))
            .flex()
            .items_center()
            .justify_end()
            // 面板空白时提示居中；有图形或轨迹时让到右下角。
            .when(idle, |el| el.top(px(9.)).justify_center())
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
                        if !trail.is_empty() {
                            let points: Vec<_> = trail
                                .iter()
                                .map(|position| {
                                    point(
                                        bounds.origin.x + position.x,
                                        bounds.origin.y + position.y,
                                    )
                                })
                                .collect();
                            paint_route(&points, 3., color, start_color, !drawing, window);
                        } else if let Some(shape) = shape {
                            // 没有手绘轨迹（载入已保存的手势）时显示保存的形状。
                            let points = place(&shape, bounds.center(), SHAPE_EXTENT);
                            paint_route(&points, 2.5, color, start_color, true, window);
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
                    } else if this.gesture_draft.trace.is_some() {
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
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(pad)
            .child(caption(
                "按住鼠标键画出相似轨迹后松开触发，不移动仍是原点击",
            ))
            .child(caption("多个手势同样相似时，在指针旁列出候选供选择"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_are_placed_inside_the_requested_extent() {
        let shape = GestureShape::from_polyline(&[(0, 0), (300, 0), (300, 120)]).unwrap();
        let center = point(px(100.), px(60.));
        let points = place(&shape, center, 80.);
        assert_eq!(points.len(), omnitrigger_core::GESTURE_SHAPE_POINTS);
        for position in &points {
            assert!((f32::from(position.x) - 100.).abs() <= 40.01);
            assert!((f32::from(position.y) - 60.).abs() <= 40.01);
        }
        assert!((f32::from(points[0].x) - 60.).abs() < 0.1);
    }

    #[test]
    fn panel_positions_are_converted_to_physical_pixels() {
        assert_eq!(physical(point(px(10.), px(20.4)), 1.5), (15, 31));
    }
}
