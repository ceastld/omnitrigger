use std::collections::BTreeMap;

use gpui::{canvas, div, fill, point, prelude::*, px, size, Bounds, Div};
use num_rational::BigRational;

use super::scatter_plot::{normalized_position, POSITION_STEPS};
use crate::theme::{self, rgb};

#[derive(PartialEq, Eq)]
pub(super) struct BoxPlot {
    quartiles: [u16; 3],
    whiskers: [u16; 2],
    outliers: Vec<u16>,
    outlier_count: u64,
    extremes: [String; 2],
    quartile_values: [String; 3],
    label: Option<String>,
}

impl BoxPlot {
    pub(super) fn new(
        [min, max]: [&BigRational; 2],
        quartiles: [&BigRational; 3],
        whiskers: [&BigRational; 2],
        outliers: &BTreeMap<&BigRational, u64>,
    ) -> Self {
        let range = max - min;
        let place = |value| normalized_position(value, min, &range);
        Self {
            quartiles: quartiles.map(place),
            whiskers: whiskers.map(place),
            outliers: outliers.keys().map(|&value| place(value)).collect(),
            outlier_count: outliers.values().sum(),
            extremes: [min.to_string(), max.to_string()],
            quartile_values: quartiles.map(ToString::to_string),
            label: None,
        }
    }

    pub(super) fn with_label(mut self, label: String) -> Self {
        self.label = (!label.trim().is_empty()).then_some(label);
        self
    }

    pub(super) fn legend(&self) -> Div {
        let count = format!("离群观测 {} 个", self.outlier_count);
        let title = match &self.label {
            Some(name) => format!("{name} · {count}"),
            None => count,
        };
        div()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(7.))
            .child(
                div()
                    .size(px(6.))
                    .flex_none()
                    .rounded(px(3.))
                    .bg(rgb(theme::ACCENT_2)),
            )
            .child(div().min_w_0().truncate().child(title))
    }

    pub(super) fn render(&self) -> Div {
        let quartiles = self.quartiles;
        let whiskers = self.whiskers;
        let outliers = self.outliers.clone();
        let outline = rgb(theme::ACCENT);
        let box_fill = rgb(theme::ACCENT_BG);
        let dot = rgb(theme::ACCENT_2);
        let plot = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let padding = px(12.);
                let start = bounds.origin.x + padding;
                let width = bounds.size.width - padding * 2.;
                let center = bounds.center().y;
                let place = |part| start + width * (f32::from(part) / f32::from(POSITION_STEPS));
                let [whisker_min, whisker_max] = whiskers.map(place);
                let [left, middle, right] = quartiles.map(place);
                let mut line = |x, y, w, h, color| {
                    window.paint_quad(fill(Bounds::new(point(x, y), size(w, h)), color));
                };
                line(
                    whisker_min,
                    center - px(1.),
                    px(f32::from(whisker_max - whisker_min).max(1.)),
                    px(2.),
                    outline,
                );
                for whisker in [whisker_min, whisker_max] {
                    line(whisker - px(1.), center - px(8.), px(2.), px(16.), outline);
                }
                let box_width = px(f32::from(right - left).max(2.));
                line(left, center - px(16.), box_width, px(32.), outline);
                if box_width > px(3.) {
                    line(
                        left + px(1.5),
                        center - px(14.5),
                        box_width - px(3.),
                        px(29.),
                        box_fill,
                    );
                }
                line(middle - px(1.), center - px(16.), px(2.), px(32.), outline);
                for &outlier in &outliers {
                    window.paint_quad(
                        fill(
                            Bounds::new(
                                point(place(outlier) - px(3.), center - px(3.)),
                                size(px(6.), px(6.)),
                            ),
                            dot,
                        )
                        .corner_radii(px(3.)),
                    );
                }
            },
        )
        .size_full();
        let labels = self.quartile_values.iter().zip(["Q1", "中位数", "Q3"]);
        let axis = div().flex().px(px(12.)).gap(px(8.));
        let axis = if self.extremes[0] == self.extremes[1] {
            axis.justify_center()
                .child(plot_label(self.extremes[0].clone()))
        } else {
            axis.justify_between()
                .child(plot_label(self.extremes[0].clone()))
                .child(plot_label(self.extremes[1].clone()))
        };
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .h(px(82.))
                    .rounded(px(5.))
                    .bg(rgb(theme::ROW))
                    .child(plot),
            )
            .child(axis)
            .child(
                div()
                    .pt(px(5.))
                    .flex()
                    .gap(px(16.))
                    .children(labels.map(|(value, label)| plot_label(format!("{label} {value}")))),
            )
    }
}

fn plot_label(label: String) -> Div {
    div()
        .min_w_0()
        .max_w(px(145.))
        .truncate()
        .text_color(rgb(theme::TEXT_MUTED))
        .text_size(px(12.))
        .child(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_plot_prepares_whiskers_and_clusters_identical_outliers() {
        let values = [1, 2, 3, 4, 8].map(|value| BigRational::from_integer(value.into()));
        let [min, q1, median, q3, max] = &values;
        let plot = BoxPlot::new(
            [min, max],
            [q1, median, q3],
            [min, q3],
            &BTreeMap::from([(max, 2)]),
        );
        assert_eq!(plot.quartiles, [585, 1170, 1755]);
        assert_eq!(plot.whiskers, [0, 1755]);
        assert_eq!(plot.outliers, [4096]);
        assert_eq!(plot.outlier_count, 2);
    }
}
