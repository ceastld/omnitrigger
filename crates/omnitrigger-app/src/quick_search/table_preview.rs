use gpui::{div, prelude::*, px, Div, FontWeight};

use crate::theme::{self, rgb};

const MAX_DATA_ROWS: usize = 6;
const MAX_VISIBLE_COLUMNS: usize = 6;
const MAX_VISIBLE_CELL_CHARACTERS: usize = 128;
const ROW_HEIGHT: f32 = 27.;

#[derive(PartialEq, Eq)]
pub(super) struct TablePreview {
    rows: Vec<Vec<String>>,
    data_rows: usize,
    columns: usize,
}

impl TablePreview {
    pub(super) fn from_rows(mut rows: Vec<Vec<String>>) -> Self {
        let data_rows = rows.len().saturating_sub(1);
        let columns = rows.first().map_or(0, Vec::len);
        rows.truncate(MAX_DATA_ROWS + 1);
        for row in &mut rows {
            row.truncate(MAX_VISIBLE_COLUMNS);
            for value in row {
                *value = display_cell(value);
            }
        }
        Self {
            rows,
            data_rows,
            columns,
        }
    }

    pub(super) fn preview_height(&self) -> f32 {
        2. + ROW_HEIGHT * self.rows.len() as f32
    }

    pub(super) fn legend(&self) -> Div {
        let data_rows = self.data_rows;
        let columns = self.columns;
        let title = if data_rows > MAX_DATA_ROWS || columns > MAX_VISIBLE_COLUMNS {
            format!(
                "{data_rows} 行 · {columns} 列（预览 {} 行、{} 列）",
                data_rows.min(MAX_DATA_ROWS),
                columns.min(MAX_VISIBLE_COLUMNS),
            )
        } else {
            format!("{data_rows} 行 · {columns} 列")
        };
        div().min_w_0().truncate().child(title)
    }

    pub(super) fn render(&self) -> Div {
        div()
            .flex()
            .flex_col()
            .w_full()
            .overflow_hidden()
            .rounded(px(6.))
            .border_1()
            .border_color(rgb(theme::BORDER_MID))
            .text_size(px(13.))
            .children(self.rows.iter().enumerate().map(|(index, row)| {
                div()
                    .flex()
                    .items_center()
                    .flex_none()
                    .h(px(ROW_HEIGHT))
                    .gap(px(1.))
                    .text_color(rgb(theme::TEXT_SOFT))
                    .when(index.is_multiple_of(2), |row| row.bg(rgb(theme::SURFACE)))
                    .when(index == 0, |row| {
                        row.bg(rgb(theme::SURFACE_HI))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme::TEXT))
                    })
                    .when(index + 1 < self.rows.len(), |row| {
                        row.border_b_1().border_color(rgb(theme::BORDER))
                    })
                    .children(row.iter().map(|value| {
                        div()
                            .flex_1()
                            .min_w_0()
                            .px(px(8.))
                            .truncate()
                            .child(value.clone())
                    }))
            }))
    }
}

fn display_cell(value: &str) -> String {
    let clipped = value.char_indices().nth(MAX_VISIBLE_CELL_CHARACTERS);
    let visible = clipped.map_or(value, |(end, _)| &value[..end]);
    let mut visible = visible
        .replace("\r\n", "↵")
        .replace(['\r', '\n'], "↵")
        .replace('\t', "⇥");
    if clipped.is_some() {
        visible.push('…');
    }
    visible
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_only_visible_records_and_columns_for_display() {
        let data = (0..10)
            .map(|row| (0..8).map(|column| format!("{row},{column}")).collect())
            .collect();
        let preview = TablePreview::from_rows(data);
        assert_eq!(preview.data_rows, 9);
        assert_eq!(preview.columns, 8);
        assert_eq!(preview.rows.len(), 7);
        assert!(preview.rows.iter().all(|row| row.len() == 6));
        assert_eq!(preview.rows[6][5], "6,5");
        assert_eq!(display_cell("带\r\n换\t行"), "带↵换⇥行");
        assert_eq!(
            display_cell(&"长".repeat(MAX_VISIBLE_CELL_CHARACTERS + 1)),
            format!("{}…", "长".repeat(MAX_VISIBLE_CELL_CHARACTERS))
        );
    }
}
