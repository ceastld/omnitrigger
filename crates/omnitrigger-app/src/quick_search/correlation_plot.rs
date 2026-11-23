use gpui::{div, prelude::*, px, Div};

use crate::theme::{self, rgb};

const ROW_HEIGHT: f32 = 24.;

#[derive(Clone, Default, PartialEq, Eq)]
pub(super) struct CorrelationCell {
    pub(super) exact: String,
    pub(super) hundredths: Option<i16>,
}

#[derive(PartialEq, Eq)]
pub(super) struct CorrelationPlot {
    names: Vec<String>,
    cells: Vec<Vec<CorrelationCell>>,
    observations: usize,
    rank: bool,
}

impl CorrelationPlot {
    pub(super) fn new(
        names: Vec<String>,
        cells: Vec<Vec<CorrelationCell>>,
        observations: usize,
        rank: bool,
    ) -> Self {
        Self {
            names,
            cells,
            observations,
            rank,
        }
    }

    pub(super) fn preview_height(&self) -> f32 {
        (self.names.len() + 1) as f32 * ROW_HEIGHT
    }

    pub(super) fn legend(&self) -> Div {
        let method = if self.rank { "Spearman" } else { "Pearson" };
        let mut label = format!("{method} · {} 条完整记录", self.observations);
        if self
            .cells
            .iter()
            .flatten()
            .any(|cell| cell.hundredths.is_none())
        {
            label.push_str(" · — 为未定义");
        }
        div().min_w_0().truncate().child(label)
    }

    pub(super) fn render(&self) -> Div {
        let headers = div()
            .flex()
            .gap(px(3.))
            .child(div().w(px(104.)).flex_none())
            .children(self.names.iter().enumerate().map(|(column, name)| {
                axis_label(name)
                    .flex_1()
                    .id(("correlation-column", column))
                    .tooltip(theme::text_tooltip(name.clone().into()))
            }));
        let rows = self.cells.iter().enumerate().map(|(row, cells)| {
            let name = &self.names[row];
            div()
                .flex()
                .gap(px(3.))
                .child(
                    axis_label(name)
                        .w(px(104.))
                        .flex_none()
                        .id(("correlation-row", row))
                        .tooltip(theme::text_tooltip(name.clone().into())),
                )
                .children(cells.iter().enumerate().map(|(column, cell)| {
                    let label = cell.hundredths.map_or_else(
                        || "—".to_owned(),
                        |hundredths| format!("{:.2}", f32::from(hundredths) / 100.),
                    );
                    let value = if cell.exact.is_empty() {
                        "未定义"
                    } else {
                        &cell.exact
                    };
                    let tooltip = format!("{name} × {}\n{value}", self.names[column]);
                    div()
                        .id(("correlation-cell", row * self.names.len() + column))
                        .h(px(ROW_HEIGHT - 3.))
                        .min_w_0()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(3.))
                        .bg(rgb(cell_background(cell.hundredths)))
                        .text_size(px(11.))
                        .text_color(rgb(theme::TEXT))
                        .child(label)
                        .tooltip(theme::text_tooltip(tooltip.into()))
                }))
        });
        div()
            .flex()
            .flex_col()
            .gap(px(3.))
            .child(headers)
            .children(rows)
    }
}

fn axis_label(name: &str) -> Div {
    div()
        .h(px(ROW_HEIGHT - 3.))
        .min_w_0()
        .px(px(3.))
        .truncate()
        .text_size(px(11.))
        .text_color(rgb(theme::TEXT_MUTED))
        .child(name.to_owned())
}

fn cell_background(hundredths: Option<i16>) -> u32 {
    let Some(value) = hundredths else {
        return theme::BACKGROUND;
    };
    let end: u32 = if value < 0 { 0x614153 } else { 0x285f55 };
    let weight = u32::from(value.unsigned_abs());
    let mix = |shift: u32| -> u32 {
        let neutral = (theme::ROW >> shift) & 0xff;
        let target = (end >> shift) & 0xff;
        let channel = (neutral * (100u32 - weight) + target * weight) / 100u32;
        channel << shift
    };
    mix(16) | mix(8) | mix(0)
}
