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
    highlighted_cells: Vec<(usize, usize)>,
}

impl TablePreview {
    pub(super) fn from_rows(rows: Vec<Vec<String>>) -> Self {
        let data_rows = rows.len().saturating_sub(1);
        let columns = rows.first().map_or(0, Vec::len);
        Self {
            rows,
            data_rows,
            columns,
            highlighted_cells: Vec::new(),
        }
    }

    pub(super) fn with_highlighted_cells(mut self, cells: Vec<(usize, usize)>) -> Self {
        self.highlighted_cells = cells;
        self
    }

    pub(super) fn is_cell_highlighted(&self, row: usize, column: usize) -> bool {
        self.highlighted_cells.binary_search(&(row, column)).is_ok()
    }

    pub(super) fn first_match_column_page(&self) -> Option<usize> {
        self.highlighted_cells
            .first()
            .map(|&(_, column)| column / MAX_VISIBLE_COLUMNS)
    }

    pub(super) fn preview_height(&self) -> f32 {
        2. + ROW_HEIGHT * self.rows.len().min(MAX_DATA_ROWS + 1) as f32
    }

    pub(super) fn row_pages(&self) -> usize {
        self.data_rows.div_ceil(MAX_DATA_ROWS)
    }

    pub(super) fn column_pages(&self) -> usize {
        self.columns.div_ceil(MAX_VISIBLE_COLUMNS)
    }

    pub(super) fn legend(&self) -> Div {
        div()
            .min_w_0()
            .truncate()
            .child(format!("{} 行 · {} 列", self.data_rows, self.columns))
    }

    fn visible_rows(&self, row_page: usize, column_page: usize) -> impl Iterator<Item = &[String]> {
        let first_record = 1 + row_page * MAX_DATA_ROWS;
        let first_column = column_page * MAX_VISIBLE_COLUMNS;
        let column_end = self.columns.min(first_column + MAX_VISIBLE_COLUMNS);
        self.rows
            .first()
            .into_iter()
            .chain(self.rows.iter().skip(first_record).take(MAX_DATA_ROWS))
            .map(move |row| &row[first_column..column_end])
    }

    pub(super) fn render(&self, row_page: usize, column_page: usize) -> Div {
        let rows = self.visible_rows(row_page, column_page);
        let visible_count = 1 + (self.data_rows - row_page * MAX_DATA_ROWS).min(MAX_DATA_ROWS);
        div()
            .flex()
            .flex_col()
            .w_full()
            .min_h(px(self.preview_height()))
            .overflow_hidden()
            .rounded(px(6.))
            .border_1()
            .border_color(rgb(theme::BORDER_MID))
            .text_size(px(13.))
            .children(rows.enumerate().map(|(index, row)| {
                let row_index = if index == 0 {
                    0
                } else {
                    row_page * MAX_DATA_ROWS + index
                };
                let first_column = column_page * MAX_VISIBLE_COLUMNS;
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
                    .when(index + 1 < visible_count, |row| {
                        row.border_b_1().border_color(rgb(theme::BORDER))
                    })
                    .children(row.iter().enumerate().map(|(column, value)| {
                        div()
                            .flex_1()
                            .min_w_0()
                            .px(px(8.))
                            .when(
                                self.is_cell_highlighted(row_index, first_column + column),
                                |cell| {
                                    cell.bg(rgb(theme::ACCENT_BG))
                                        .text_color(rgb(theme::ACCENT))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .rounded(px(4.))
                                },
                            )
                            .truncate()
                            .child(display_cell(value))
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
    fn pages_records_and_columns_without_losing_the_header_or_last_cells() {
        let data = (0..10)
            .map(|row| (0..8).map(|column| format!("{row},{column}")).collect())
            .collect();
        let preview = TablePreview::from_rows(data);
        assert_eq!(preview.row_pages(), 2);
        assert_eq!(preview.column_pages(), 2);
        let first: Vec<_> = preview.visible_rows(0, 0).collect();
        assert_eq!(first.len(), 7);
        assert!(first.iter().all(|row| row.len() == 6));
        assert_eq!(first[6][5], "6,5");
        let last: Vec<_> = preview.visible_rows(1, 1).collect();
        assert_eq!(last.len(), 4);
        assert_eq!(last[0], ["0,6", "0,7"]);
        assert_eq!(last[1], ["7,6", "7,7"]);
        assert_eq!(last[3], ["9,6", "9,7"]);
        let header_only = TablePreview::from_rows(vec![vec!["列名".into()]]);
        assert_eq!(header_only.row_pages(), 0);
        assert_eq!(header_only.visible_rows(0, 0).count(), 1);
        assert_eq!(display_cell("带\r\n换\t行"), "带↵换⇥行");
        assert_eq!(
            display_cell(&"长".repeat(MAX_VISIBLE_CELL_CHARACTERS + 1)),
            format!("{}…", "长".repeat(MAX_VISIBLE_CELL_CHARACTERS))
        );
    }
}
