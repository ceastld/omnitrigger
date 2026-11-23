use gpui::{canvas, div, fill, point, prelude::*, px, size, Bounds, Div, PathBuilder};
use num_bigint::BigInt;
use num_rational::BigRational;

use crate::theme::{self, rgb};

use super::calculator::format_round_trip;
use super::fraction::format_significant_digits;

pub(super) const PREVIEW_HEIGHT: f32 = 134.;
const AXIS_WIDTH: f32 = 81.;
const MAX_AXIS_LABEL_WIDTH: f32 = 167.;
const AXIS_GAP: f32 = 8.;
const SQUARE_AXIS_LABEL_OVERHANG: f32 = 112.;
pub(super) const POSITION_STEPS: u16 = 4096;
const PLOT_PADDING: f32 = 9.;
const SAMPLE_SERIES_COLORS: [u32; 4] = [
    theme::ACCENT,
    theme::ACCENT_2,
    theme::DANGER,
    theme::TEXT_MUTED,
];

pub(super) struct SampledSeries {
    pub(super) label: String,
    pub(super) points: Vec<(BigRational, BigRational)>,
}

#[derive(PartialEq, Eq)]
pub(super) struct ScatterPlot {
    locations: Vec<(u16, u16)>,
    curve: Vec<(u16, u16)>,
    x: [String; 2],
    y: [String; 2],
    axis_names: [String; 2],
    zero_axes: [Option<u16>; 2],
    kind: ScatterKind,
    sample_series: Vec<(String, usize)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScatterKind {
    Observations,
    Residuals,
    Cumulative { observations: u64 },
    Function,
    SampledFunction { requested: usize },
    Parametric { requested: usize },
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

    pub(super) fn from_parametric_samples(
        points: &[(BigRational, BigRational)],
        requested: usize,
    ) -> Self {
        Self::prepare(points, &[], ScatterKind::Parametric { requested }, None)
    }

    pub(super) fn from_sampled_series(
        series: Vec<SampledSeries>,
        x_bounds: [&BigRational; 2],
        requested: usize,
    ) -> Self {
        let mut points = Vec::new();
        let mut labels = Vec::with_capacity(series.len());
        for SampledSeries {
            label,
            points: series_points,
        } in series
        {
            labels.push((label, series_points.len()));
            points.extend(series_points);
        }
        let mut plot = Self::from_sampled_function(&points, x_bounds, requested);
        plot.sample_series = labels;
        plot
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
        let measured = [[min_x, max_x], [min_y, max_y]];
        let square =
            matches!(kind, ScatterKind::Parametric { .. }).then(|| square_plot_bounds(measured));
        let [[min_x, max_x], [min_y, max_y]] = square
            .as_ref()
            .map_or(measured, |axes| axes.each_ref().map(|axis| axis.each_ref()));
        let range_x = max_x - min_x;
        let range_y = max_y - min_y;
        let zero_axes = if matches!(
            kind,
            ScatterKind::Function
                | ScatterKind::SampledFunction { .. }
                | ScatterKind::Parametric { .. }
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
        let label = |value: &BigRational| {
            if square.is_none() {
                return value.to_string();
            }
            let rounded = format_significant_digits(value.clone(), 17);
            rounded
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
                .map_or(rounded, format_round_trip)
        };
        Self {
            locations: points.iter().map(place).collect(),
            curve: curve.iter().map(place).collect(),
            x: [min_x, max_x].map(label),
            y: [min_y, max_y].map(label),
            axis_names: match kind {
                ScatterKind::Observations
                | ScatterKind::Function
                | ScatterKind::Parametric { .. } => ["x", "y"],
                ScatterKind::SampledFunction { .. } => ["x", "近似值"],
                ScatterKind::Residuals => ["x", "残差"],
                ScatterKind::Cumulative { .. } => ["观测值", "累计频率"],
            }
            .map(str::to_owned),
            zero_axes,
            kind,
            sample_series: Vec::new(),
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
        if !self.sample_series.is_empty() {
            let mut legend = div().flex().min_w_0().items_center().gap(px(11.));
            for (index, (label, _)) in self.sample_series.iter().enumerate() {
                let color = rgb(SAMPLE_SERIES_COLORS[index % SAMPLE_SERIES_COLORS.len()]);
                legend = legend.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .child(div().flex_none().size(px(6.)).rounded(px(3.)).bg(color))
                        .child(div().min_w_0().truncate().child(label.clone())),
                );
            }
            return legend;
        }
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
            ScatterKind::Parametric { requested } => {
                (format!("参数采样 · {points}/{requested} 个有效点"), "")
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
        let colors = self
            .sample_series
            .iter()
            .enumerate()
            .flat_map(|(index, (_, count))| {
                let color = rgb(SAMPLE_SERIES_COLORS[index % SAMPLE_SERIES_COLORS.len()]);
                std::iter::repeat_n(color, *count)
            });
        let points: Vec<_> = self
            .locations
            .iter()
            .copied()
            .zip(colors.chain(std::iter::repeat(rgb(theme::ACCENT))))
            .collect();
        let curve = self.curve.clone();
        let grid = rgb(theme::BORDER_MID);
        let [zero_x, zero_y] = self.zero_axes;
        let axis = rgb(theme::BORDER_HI);
        let line = rgb(theme::ACCENT_2);
        let square = matches!(self.kind, ScatterKind::Parametric { .. });
        let marker = px(if square { 2. } else { 3. });
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
                for (position, color) in &points {
                    let center = place(position);
                    window.paint_quad(
                        fill(
                            Bounds::new(
                                center - point(marker, marker),
                                size(marker * 2., marker * 2.),
                            ),
                            *color,
                        )
                        .corner_radii(marker),
                    );
                }
            },
        )
        .size_full();

        let vertical = div()
            .min_w(px(if square {
                SQUARE_AXIS_LABEL_OVERHANG
            } else {
                AXIS_WIDTH
            }))
            .max_w(px(MAX_AXIS_LABEL_WIDTH))
            .h(px(PREVIEW_HEIGHT))
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
            .when(square, |labels| {
                labels
                    .w(px(PREVIEW_HEIGHT + 2. * SQUARE_AXIS_LABEL_OVERHANG))
                    .ml(px(-SQUARE_AXIS_LABEL_OVERHANG))
            })
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
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(axis_label(self.x[0].clone())),
                )
                .child(axis_label(x_label.clone()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .justify_end()
                        .child(axis_label(self.x[1].clone())),
                )
        };
        let axes = div()
            .when(square, |axes| axes.pr(px(SQUARE_AXIS_LABEL_OVERHANG)))
            .flex()
            .gap(px(AXIS_GAP))
            .child(vertical)
            .child(
                div()
                    .when(square, |body| body.w(px(PREVIEW_HEIGHT)).flex_none())
                    .when(!square, |body| body.flex_1().min_w_0())
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .h(px(PREVIEW_HEIGHT))
                            .min_w_0()
                            .rounded(px(5.))
                            .bg(rgb(theme::ROW))
                            .child(plot),
                    )
                    .child(horizontal),
            );
        if square {
            div().w_full().flex().justify_center().child(axes)
        } else {
            axes
        }
    }
}

fn square_plot_bounds(measured: [[&BigRational; 2]; 2]) -> [[BigRational; 2]; 2] {
    let side = measured
        .map(|[minimum, maximum]| maximum - minimum)
        .into_iter()
        .max()
        .expect("平面图包含两个轴");
    let limit = BigRational::from_float(f64::MAX).expect("最大浮点数为有限值");
    measured.map(|[minimum, maximum]| {
        let padding = (&side - (maximum - minimum)) / BigInt::from(2u8);
        let lower = (minimum - padding).clamp(-&limit, &limit - &side);
        let upper = &lower + &side;
        [lower, upper]
    })
}

fn axis_label(text: String) -> Div {
    div()
        .min_w_0()
        .max_w(px(MAX_AXIS_LABEL_WIDTH))
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
    fn parametric_plot_keeps_the_same_unit_on_both_axes() {
        let points = [(0, 0), (1, 0), (2, 1)].map(|(x, y)| {
            (
                BigRational::from_integer(x.into()),
                BigRational::from_integer(y.into()),
            )
        });
        let plot = ScatterPlot::from_parametric_samples(&points, 3);
        assert_eq!(plot.x, ["0", "2"]);
        assert_eq!(plot.y, ["-0.5", "1.5"]);
        assert_eq!(plot.locations, [(0, 1024), (2048, 1024), (4096, 3072)]);

        let limit = BigRational::from_float(f64::MAX).unwrap();
        let points = [(BigRational::default(), -&limit), (limit.clone(), limit)];
        let plot = ScatterPlot::from_parametric_samples(&points, 2);
        assert_eq!(plot.locations, [(2048, 0), (4096, 4096)]);
        assert_eq!(plot.x, plot.y);
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
