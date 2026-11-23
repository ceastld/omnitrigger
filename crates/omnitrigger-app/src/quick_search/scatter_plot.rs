use gpui::{canvas, div, fill, point, prelude::*, px, size, Bounds, Div, PathBuilder};
use num_bigint::BigInt;
use num_rational::BigRational;

use crate::theme::{self, rgb};

pub(super) const PREVIEW_HEIGHT: f32 = 134.;
const AXIS_WIDTH: f32 = 81.;
const AXIS_GAP: f32 = 8.;
pub(super) const POSITION_STEPS: u16 = 4096;
const PLOT_PADDING: f32 = 9.;

#[derive(PartialEq, Eq)]
pub(super) struct ScatterPlot {
    locations: Vec<(u16, u16)>,
    curve: Vec<(u16, u16)>,
    x: [String; 2],
    y: [String; 2],
    axis_names: [String; 2],
    zero_axes: [Option<u16>; 2],
    kind: ScatterKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScatterKind {
    Observations,
    Residuals,
    Cumulative { observations: u64 },
    Function,
    SampledFunction { requested: usize },
}

impl ScatterPlot {
    pub(super) fn from_points(points: &[(BigRational, BigRational)]) -> Self {
        Self::from_series(points, &[])
    }

    pub(super) fn from_series(
        points: &[(BigRational, BigRational)],
        curve: &[(BigRational, BigRational)],
    ) -> Self {
        Self::prepare(points, curve, ScatterKind::Observations, None)
    }

    pub(super) fn from_residuals(points: &[(BigRational, BigRational)]) -> Self {
        let xs = points.iter().map(|(x, _)| x);
        let min = xs.clone().min().expect("残差图至少有两个数据点");
        let max = xs.max().expect("残差图至少有两个数据点");
        let mut baseline = vec![(min.clone(), BigRational::default())];
        if min != max {
            baseline.push((max.clone(), BigRational::default()));
        }
        Self::prepare(points, &baseline, ScatterKind::Residuals, None)
    }

    pub(super) fn from_cumulative_distribution(
        points: &[(BigRational, BigRational)],
        observations: u64,
    ) -> Self {
        let mut curve = Vec::with_capacity(points.len() * 2);
        let mut previous = BigRational::default();
        for (value, frequency) in points {
            curve.push((value.clone(), previous));
            curve.push((value.clone(), frequency.clone()));
            previous = frequency.clone();
        }
        Self::prepare(
            points,
            &curve,
            ScatterKind::Cumulative { observations },
            None,
        )
    }

    pub(super) fn from_function_curve(points: &[(BigRational, BigRational)]) -> Self {
        Self::prepare(&[], points, ScatterKind::Function, None)
    }

    pub(super) fn from_sampled_function(
        points: &[(BigRational, BigRational)],
        x_bounds: [&BigRational; 2],
        requested: usize,
    ) -> Self {
        Self::prepare(
            points,
            &[],
            ScatterKind::SampledFunction { requested },
            Some(x_bounds),
        )
    }

    fn prepare(
        points: &[(BigRational, BigRational)],
        curve: &[(BigRational, BigRational)],
        kind: ScatterKind,
        x_bounds: Option<[&BigRational; 2]>,
    ) -> Self {
        let [min_x, max_x] = x_bounds.unwrap_or_else(|| {
            let x = points.iter().chain(curve).map(|(x, _)| x);
            [
                x.clone().min().expect("图形至少有一个数据点"),
                x.max().expect("图形至少有一个数据点"),
            ]
        });
        let y = points.iter().chain(curve).map(|(_, y)| y);
        let min_y = y.clone().min().expect("图形至少有一个数据点");
        let max_y = y.max().expect("图形至少有一个数据点");
        let range_x = max_x - min_x;
        let range_y = max_y - min_y;
        let zero_axes = if matches!(
            kind,
            ScatterKind::Function | ScatterKind::SampledFunction { .. }
        ) {
            let zero = BigRational::default();
            [
                (min_x <= &zero && &zero <= max_x)
                    .then(|| normalized_position(&zero, min_x, &range_x)),
                (min_y <= &zero && &zero <= max_y)
                    .then(|| normalized_position(&zero, min_y, &range_y)),
            ]
        } else {
            [None; 2]
        };
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
            axis_names: match kind {
                ScatterKind::Observations | ScatterKind::Function => ["x", "y"],
                ScatterKind::SampledFunction { .. } => ["x", "近似值"],
                ScatterKind::Residuals => ["x", "残差"],
                ScatterKind::Cumulative { .. } => ["观测值", "累计频率"],
            }
            .map(str::to_owned),
            zero_axes,
            kind,
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

    pub(super) fn with_range_labels(mut self, x: [String; 2], y: [String; 2]) -> Self {
        self.x = x;
        self.y = y;
        self
    }

    pub(super) fn legend(&self) -> Div {
        let points = self.point_count();
        let (count, line_label) = match self.kind {
            ScatterKind::Observations => (format!("{points} 个观测点"), "拟合曲线"),
            ScatterKind::Residuals => (format!("{points} 个残差点"), "零残差"),
            ScatterKind::Cumulative { observations } => (
                format!("{observations} 个观测 · {points} 个不同值"),
                "累计频率",
            ),
            ScatterKind::Function => (format!("{} 个采样点", self.curve.len()), "函数曲线"),
            ScatterKind::SampledFunction { requested } => {
                (format!("近似采样 · {points}/{requested} 个有效点"), "")
            }
        };
        div()
            .flex()
            .items_center()
            .gap(px(7.))
            .when(self.kind != ScatterKind::Function, |legend| {
                legend.child(div().size(px(6.)).rounded(px(3.)).bg(rgb(theme::ACCENT)))
            })
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
        let [zero_x, zero_y] = self.zero_axes;
        let axis = rgb(theme::BORDER_HI);
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
                if let Some(x) = zero_x {
                    let x = place(&(x, 0)).x;
                    window.paint_quad(fill(
                        Bounds::new(point(x, origin.y), size(px(1.5), height)),
                        axis,
                    ));
                }
                if let Some(y) = zero_y {
                    let y = place(&(0, y)).y;
                    window.paint_quad(fill(
                        Bounds::new(point(origin.x, y), size(width, px(1.5))),
                        axis,
                    ));
                }
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

pub(super) fn normalized_position(
    value: &BigRational,
    min: &BigRational,
    range: &BigRational,
) -> u16 {
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
    fn preserves_the_sampled_function_x_range_when_endpoint_values_are_missing() {
        let axis: [BigRational; 2] = [
            BigRational::from_integer((-1).into()),
            BigRational::from_integer(1.into()),
        ];
        let points = [(BigRational::default(), BigRational::default())];
        let plot = ScatterPlot::from_sampled_function(&points, [&axis[0], &axis[1]], 3);
        assert_eq!(plot.x, ["-1", "1"]);
        assert_eq!(plot.locations, [(POSITION_STEPS / 2, POSITION_STEPS / 2)]);
        assert_eq!(plot.zero_axes[0], Some(POSITION_STEPS / 2));
        assert!(!plot.has_curve());
    }

    #[test]
    fn function_plot_positions_only_the_zero_axes_inside_its_range() {
        let points = [(-2, 1), (2, 3)].map(|(x, y)| {
            (
                BigRational::from_integer(x.into()),
                BigRational::from_integer(y.into()),
            )
        });
        assert_eq!(
            ScatterPlot::from_function_curve(&points).zero_axes,
            [Some(2048), None]
        );
        assert_eq!(ScatterPlot::from_points(&points).zero_axes, [None; 2]);
    }

    #[test]
    fn cumulative_plot_draws_each_jump_vertically_with_a_closed_point_at_the_top() {
        let points = [
            (BigRational::default(), BigRational::new(1.into(), 2.into())),
            (
                BigRational::from_integer(2.into()),
                BigRational::from_integer(1.into()),
            ),
        ];
        let plot = ScatterPlot::from_cumulative_distribution(&points, 4);
        assert_eq!(plot.curve, [(0, 0), (0, 2048), (4096, 2048), (4096, 4096)]);
        assert_eq!(plot.locations, [(0, 2048), (4096, 4096)]);
        assert_eq!(plot.y, ["0", "1"]);
        let single = ScatterPlot::from_cumulative_distribution(&points[1..], 3);
        assert_eq!(single.curve, [(2048, 0), (2048, 4096)]);
        assert_eq!(single.locations, [(2048, 4096)]);
    }

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
