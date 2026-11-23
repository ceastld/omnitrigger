use gpui::{div, prelude::*, px, Div};
use num_rational::BigRational;

use crate::theme::{self, rgb};

const BAR_SECTION: u32 = 96;

#[derive(PartialEq, Eq)]
pub(super) struct BinPlot {
    edges: Vec<String>,
    bins: Vec<u64>,
    count: u64,
}

impl BinPlot {
    pub(super) fn new(edges: Vec<String>, bins: Vec<u64>, count: u64) -> Self {
        Self { edges, bins, count }
    }

    pub(super) fn bin_count(&self) -> usize {
        self.bins.len()
    }

    pub(super) fn legend(&self) -> Div {
        div().child(format!(
            "{} 个观测 · {} 个区间",
            self.count,
            self.bin_count()
        ))
    }

    pub(super) fn render(&self) -> Div {
        let max = self.bins.iter().copied().max().expect("分箱至少有两个区间");
        let bars =
            self.edges
                .windows(2)
                .zip(&self.bins)
                .enumerate()
                .map(|(index, (edges, &count))| {
                    let frequency = BigRational::new(count.into(), self.count.into());
                    let tooltip = format!(
                        "{} ≤ x < {}\n次数 {} · 频率 {frequency}",
                        edges[0], edges[1], count
                    );
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
                                .h(px(bar_height(count, max) as f32))
                                .rounded(px(3.))
                                .bg(rgb(theme::ACCENT)),
                        )
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
                            .child(label(max.to_string()))
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
                div()
                    .ml(px(93.))
                    .flex()
                    .justify_center()
                    .child(label("区间从小到大".to_owned())),
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

fn bar_height(count: u64, max: u64) -> u32 {
    if count == 0 {
        return 0;
    }
    let height = u128::from(count) * u128::from(BAR_SECTION) / u128::from(max);
    (height.max(1)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_empty_bins_and_handles_the_largest_counts() {
        assert_eq!(bar_height(0, u64::MAX), 0);
        assert_eq!(bar_height(1, u64::MAX), 1);
        assert_eq!(bar_height(u64::MAX, u64::MAX), BAR_SECTION);
    }
}
