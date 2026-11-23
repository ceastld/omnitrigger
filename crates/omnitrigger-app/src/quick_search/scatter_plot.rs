use gpui::{canvas, div, fill, point, prelude::*, px, size, Bounds, Div, PathBuilder};
use num_bigint::BigInt;
use num_rational::BigRational;

use crate::theme::{self, rgb};

pub(super) const PREVIEW_HEIGHT: f32 = 134.;
const AXIS_WIDTH: f32 = 81.;
const AXIS_GAP: f32 = 8.;
const POSITION_STEPS: u16 = 4096;
const PLOT_PADDING: f32 = 9.;

#[derive(PartialEq, Eq)]
pub(super) struct ScatterPlot {
    locations: Vec<(u16, u16)>,
    curve: Vec<(u16, u16)>,
    x: [String; 2],
    y: [String; 2],
    axis_names: [String; 2],
    residuals: bool,
}

impl ScatterPlot {
    pub(super) fn from_points(points: &[(BigRational, BigRational)]) -> Self {
        Self::from_series(points, &[])
    }

    pub(super) fn from_series(
        points: &[(BigRational, BigRational)],
        curve: &[(BigRational, BigRational)],
    ) -> Self {
        Self::prepare(points, curve, false)
    }

    pub(super) fn from_residuals(points: &[(BigRational, BigRational)]) -> Self {
        let xs = points.iter().map(|(x, _)| x);
        let min = xs.clone().min().expect("残差图至少有两个数据点");
        let max = xs.max().expect("残差图至少有两个数据点");
        let mut baseline = vec![(min.clone(), BigRational::default())];
        if min != max {
            baseline.push((max.clone(), BigRational::default()));
        }
        Self::prepare(points, &baseline, true)
    }

    fn prepare(
        points: &[(BigRational, BigRational)],
        curve: &[(BigRational, BigRational)],
        residuals: bool,
    ) -> Self {
        let x = points.iter().chain(curve).map(|(x, _)| x);
        let y = points.iter().chain(curve).map(|(_, y)| y);
        let min_x = x.clone().min().expect("散点图至少有两个数据点");
        let max_x = x.max().expect("散点图至少有两个数据点");
        let min_y = y.clone().min().expect("散点图至少有两个数据点");
        let max_y = y.max().expect("散点图至少有两个数据点");
        let range_x = max_x - min_x;
        let range_y = max_y - min_y;
        let place = |(x, y): &(BigRational, BigRational)| {
            (
                normalized_position(x, min_x, &range_x),
                normalized_position(y, min_y, &range_y),
            )
        };
        Self {
            locations: points.iter().map(place).collect(),
            curve: curve.iter().map(place).collect(),
            x: [min_x.to_string(), max_x.to_string()],
            y: [min_y.to_string(), max_y.to_string()],
            axis_names: [
                "x".to_owned(),
                if residuals { "残差" } else { "y" }.to_owned(),
            ],
            residuals,
        }
    }

    pub(super) fn point_count(&self) -> usize {
        self.locations.len()
    }

    pub(super) fn with_axis_names(mut self, names: [String; 2]) -> Self {
        for (label, name) in self.axis_names.iter_mut().zip(names) {
            if !name.trim().is_empty() {
                *label = name;
            }
        }
        self
    }

    pub(super) fn has_curve(&self) -> bool {
        !self.curve.is_empty()
    }

    pub(super) fn legend(&self) -> Div {
        let point_label = if self.residuals {
            "残差点"
        } else {
            "观测点"
        };
        let line_label = if self.residuals {
            "零残差"
        } else {
            "拟合曲线"
        };
        let count = format!("{} 个{point_label}", self.point_count());
        div()
            .flex()
            .items_center()
            .gap(px(7.))
            .child(div().size(px(6.)).rounded(px(3.)).bg(rgb(theme::ACCENT)))
            .child(count)
            .when(self.has_curve(), |legend| {
                legend
                    .child(
                        div()
                            .ml(px(5.))
                            .w(px(14.))
                            .h(px(2.))
                            .bg(rgb(theme::ACCENT_2)),
                    )
                    .child(line_label)
            })
    }

    pub(super) fn render(&self) -> Div {
        let points = self.locations.clone();
        let curve = self.curve.clone();
        let grid = rgb(theme::BORDER_MID);
        let dot = rgb(theme::ACCENT);
        let line = rgb(theme::ACCENT_2);
        let plot = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let padding = px(PLOT_PADDING);
                let origin = bounds.origin + point(padding, padding);
                let width = bounds.size.width - padding * 2.;
                let height = bounds.size.height - padding * 2.;
                for tick in [0., 0.25, 0.5, 0.75, 1.] {
                    window.paint_quad(fill(
                        Bounds::new(
                            point(origin.x + width * tick, origin.y),
                            size(px(1.), height),
                        ),
                        grid,
                    ));
                    window.paint_quad(fill(
                        Bounds::new(
                            point(origin.x, origin.y + height * tick),
                            size(width, px(1.)),
                        ),
                        grid,
                    ));
                }
                let place = |&(x, y): &(u16, u16)| {
                    let x = f32::from(x) / f32::from(POSITION_STEPS);
                    let y = f32::from(y) / f32::from(POSITION_STEPS);
                    point(origin.x + width * x, origin.y + height * (1. - y))
                };
                if let Some((first, rest)) = curve.split_first() {
                    let start = place(first);
                    if rest.is_empty() {
                        window.paint_quad(fill(
                            Bounds::new(start - point(px(5.), px(1.)), size(px(10.), px(2.))),
                            line,
                        ));
                    } else {
                        let mut path = PathBuilder::stroke(px(2.));
                        path.move_to(start);
                        for point in rest {
                            path.line_to(place(point));
                        }
                        if let Ok(path) = path.build() {
                            window.paint_path(path, line);
                        }
                    }
                }
                for center in points.iter().map(place) {
                    window.paint_quad(
                        fill(
                            Bounds::new(center - point(px(3.), px(3.)), size(px(6.), px(6.))),
                            dot,
                        )
                        .corner_radii(px(3.)),
                    );
                }
            },
        )
        .size_full();

        let vertical = div()
            .w(px(AXIS_WIDTH))
            .flex_none()
            .flex()
            .flex_col()
            .items_end()
            .py(px(3.));
        let [x_label, y_label] = &self.axis_names;
        let vertical = if self.y[0] == self.y[1] {
            vertical
                .justify_center()
                .child(axis_label(format!("{y_label} = {}", self.y[0])))
        } else {
            vertical
                .justify_between()
                .child(axis_label(self.y[1].clone()))
                .child(axis_label(y_label.clone()))
                .child(axis_label(self.y[0].clone()))
        };
        let horizontal = div()
            .ml(px(AXIS_WIDTH + AXIS_GAP))
            .px(px(PLOT_PADDING))
            .h(px(19.))
            .flex()
            .items_end()
            .gap(px(8.));
        let horizontal = if self.x[0] == self.x[1] {
            horizontal
                .justify_center()
                .child(axis_label(format!("{x_label} = {}", self.x[0])))
        } else {
            horizontal
                .justify_between()
                .child(axis_label(self.x[0].clone()))
                .child(axis_label(x_label.clone()))
                .child(axis_label(self.x[1].clone()))
        };
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(PREVIEW_HEIGHT))
                    .flex()
                    .gap(px(AXIS_GAP))
                    .child(vertical)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .rounded(px(5.))
                            .bg(rgb(theme::ROW))
                            .child(plot),
                    ),
            )
            .child(horizontal)
    }
}

fn axis_label(text: String) -> Div {
    div()
        .max_w(px(AXIS_WIDTH))
        .text_size(px(11.))
        .font_family(theme::MONO_FONT)
        .text_color(rgb(theme::TEXT_MUTED))
        .truncate()
        .child(text)
}

fn normalized_position(value: &BigRational, min: &BigRational, range: &BigRational) -> u16 {
    if range.numer().sign() == num_bigint::Sign::NoSign {
        return POSITION_STEPS / 2;
    }
    let units = (value - min) / range * BigInt::from(POSITION_STEPS);
    u16::try_from(units.to_integer()).expect("坐标在计算出的范围内")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn residual_plot_includes_a_zero_baseline_and_keeps_the_signs() {
        let points = [(-1, 0), (0, -1), (1, 0), (0, 1)].map(|(x, y)| {
            (
                BigRational::from_integer(x.into()),
                BigRational::from_integer(y.into()),
            )
        });
        let plot = ScatterPlot::from_residuals(&points);
        assert_eq!(
            plot.locations,
            [(0, 2048), (2048, 0), (4096, 2048), (2048, 4096)]
        );
        assert_eq!(plot.curve, [(0, 2048), (4096, 2048)]);
        assert_eq!(plot.y, ["-1", "1"]);
    }

    #[test]
    fn scales_large_close_values_and_places_constant_axes_in_the_middle() {
        let base = BigInt::from(10u8).pow(100);
        let point = |offset: u8| {
            (
                BigRational::from_integer(&base + offset),
                BigRational::new(1.into(), 3.into()),
            )
        };
        let plot = ScatterPlot::from_points(&[point(4), point(1), point(2)]);
        assert_eq!(plot.locations, [(4096, 2048), (0, 2048), (1365, 2048)]);
        assert_eq!(plot.y, ["1/3", "1/3"]);
    }
}
