use gpui::{div, prelude::*, px, Div};
use num_rational::BigRational;

use crate::theme::{self, rgb};

const BAR_SECTION: u32 = 96;
const CATEGORY_CAPTION: u32 = 20;

#[derive(PartialEq, Eq)]
pub(super) struct BinPlot {
    labels: BarLabels,
    bins: Vec<u64>,
    count: u64,
    label: Option<String>,
}

#[derive(PartialEq, Eq)]
enum BarLabels {
    Intervals(Vec<String>),
    Categories { visible: Vec<String>, total: usize },
}

impl BinPlot {
    pub(super) fn new(edges: Vec<String>, bins: Vec<u64>, count: u64) -> Self {
        Self {
            labels: BarLabels::Intervals(edges),
            bins,
            count,
            label: None,
        }
    }

    pub(super) fn from_categories(
        labels: Vec<String>,
        bins: Vec<u64>,
        count: u64,
        total_categories: usize,
    ) -> Self {
        debug_assert_eq!(labels.len(), bins.len());
        Self {
            labels: BarLabels::Categories {
                visible: labels,
                total: total_categories,
            },
            bins,
            count,
            label: None,
        }
    }

    pub(super) fn with_label(mut self, label: String) -> Self {
        self.label = (!label.trim().is_empty()).then_some(label);
        self
    }

    pub(super) fn bin_count(&self) -> usize {
        self.bins.len()
    }

    pub(super) fn legend(&self) -> Div {
        let classes = match &self.labels {
            BarLabels::Intervals(_) => format!("{} 个区间", self.bin_count()),
            BarLabels::Categories { visible, total } if visible.len() < *total => {
                format!("前 {} 类 / 共 {total} 类", visible.len())
            }
            BarLabels::Categories { total, .. } => format!("{total} 类"),
        };
        let counts = format!("{} 个观测 · {classes}", self.count);
        let title = match &self.label {
            Some(name) => format!("{name} · {counts}"),
            None => counts,
        };
        div().min_w_0().truncate().child(title)
    }

    pub(super) fn render(&self) -> Div {
        let max = self
            .bins
            .iter()
            .copied()
            .max()
            .expect("次数图至少有一个条目");
        let max_label = if max < 1_000_000_000_000 {
            max.to_string()
        } else {
            format!("≈{:.2e}", max as f64)
        };
        let categorical = matches!(&self.labels, BarLabels::Categories { .. });
        let caption_space = if categorical { CATEGORY_CAPTION } else { 0 };
        let bars = self.bins.iter().enumerate().map(|(index, &count)| {
            let (description, caption) = match &self.labels {
                BarLabels::Intervals(edges) => {
                    (format!("{} ≤ x < {}", edges[index], edges[index + 1]), None)
                }
                BarLabels::Categories { visible, .. } => {
                    let caption = &visible[index];
                    (caption.clone(), Some(caption.replace(['\r', '\n'], " ")))
                }
            };
            let frequency = BigRational::new(count.into(), self.count.into());
            let tooltip = format!("{description}\n次数 {count} · 频率 {frequency}");
            div()
                .id(("bin-bar", index))
                .h_full()
                .min_w_0()
                .flex_1()
                .flex()
                .flex_col()
                .justify_end()
                .child(
                    div()
                        .w_full()
                        .h(px(
                            bar_height(count, max, BAR_SECTION - caption_space) as f32
                        ))
                        .flex_none()
                        .rounded(px(3.))
                        .bg(rgb(theme::ACCENT)),
                )
                .when_some(caption, |bar, caption| {
                    bar.child(label(caption).h(px(caption_space as f32)).pt(px(3.)))
                })
                .tooltip(theme::text_tooltip(tooltip.into()))
        });

        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .h(px(116.))
                    .gap(px(8.))
                    .child(
                        div()
                            .w(px(85.))
                            .flex_none()
                            .flex()
                            .flex_col()
                            .items_end()
                            .justify_between()
                            .py(px(4.))
                            .when(categorical, |scale| {
                                scale.pb(px((4 + caption_space) as f32))
                            })
                            .child(label(max_label))
                            .child(label("次数".to_owned()))
                            .child(label("0".to_owned())),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .gap(px(4.))
                            .items_end()
                            .p(px(10.))
                            .rounded(px(5.))
                            .bg(rgb(theme::ROW))
                            .children(bars),
                    ),
            )
            .child(
                div().ml(px(93.)).flex().justify_center().child(label(
                    if categorical {
                        "次数从高到低"
                    } else {
                        "区间从小到大"
                    }
                    .to_owned(),
                )),
            )
    }
}

fn label(text: String) -> Div {
    div()
        .max_w_full()
        .truncate()
        .text_color(rgb(theme::TEXT_MUTED))
        .text_size(px(11.))
        .child(text)
}

fn bar_height(count: u64, max: u64, section: u32) -> u32 {
    if count == 0 {
        return 0;
    }
    let height = u128::from(count) * u128::from(section) / u128::from(max);
    (height.max(1)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_empty_bins_and_handles_the_largest_counts() {
        assert_eq!(bar_height(0, u64::MAX, BAR_SECTION), 0);
        assert_eq!(bar_height(1, u64::MAX, BAR_SECTION), 1);
        assert_eq!(bar_height(u64::MAX, u64::MAX, BAR_SECTION), BAR_SECTION);
        assert_eq!(bar_height(10, 10, BAR_SECTION - CATEGORY_CAPTION), 76);
    }
}
