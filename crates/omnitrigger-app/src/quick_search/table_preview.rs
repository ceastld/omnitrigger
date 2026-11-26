use std::ops::Range;

use gpui::{div, prelude::*, px, relative, Div, FontWeight};

use crate::theme::{self, rgb};

use super::csv_tools::write_rows;

pub(super) const MAX_DATA_ROWS: usize = 6;
pub(super) const MAX_VISIBLE_COLUMNS: usize = 6;
const MAX_VISIBLE_CELL_CHARACTERS: usize = 128;
const MAX_COLUMN_WIDTH_UNITS: usize = 26;
const MIN_COLUMN_WIDTH_UNITS: usize = 6;
const ROW_HEIGHT: f32 = 27.;

#[derive(PartialEq, Eq)]
pub(super) struct TablePreview {
    rows: Vec<Vec<String>>,
    data_rows: usize,
    columns: usize,
    columns_per_page: usize,
    column_width_units: Vec<usize>,
    highlighted_cells: Vec<(usize, usize)>,
    input_errors: Vec<Range<usize>>,
}

impl TablePreview {
    pub(super) fn from_rows(rows: Vec<Vec<String>>) -> Self {
        let data_rows = rows.len().saturating_sub(1);
        let columns = rows.first().map_or(0, Vec::len);
        let mut column_width_units = vec![MIN_COLUMN_WIDTH_UNITS; columns];
        for row in &rows {
            for (width, value) in column_width_units.iter_mut().zip(row) {
                *width = (*width).max(cell_width_units(value));
            }
        }
        Self {
            rows,
            data_rows,
            columns,
            columns_per_page: MAX_VISIBLE_COLUMNS,
            column_width_units,
            highlighted_cells: Vec::new(),
            input_errors: Vec::new(),
        }
    }

    pub(super) fn with_columns_per_page(mut self, columns: usize) -> Self {
        self.columns_per_page = columns.clamp(1, MAX_VISIBLE_COLUMNS);
        self
    }

    pub(super) fn with_highlighted_cells(mut self, cells: Vec<(usize, usize)>) -> Self {
        self.highlighted_cells = cells;
        self
    }

    pub(super) fn with_input_errors(mut self, errors: Vec<Range<usize>>) -> Self {
        self.input_errors = errors;
        self
    }

    pub(super) fn input_errors(&self) -> &[Range<usize>] {
        &self.input_errors
    }

    pub(super) fn is_cell_highlighted(&self, row: usize, column: usize) -> bool {
        self.highlighted_cells.binary_search(&(row, column)).is_ok()
    }

    pub(super) fn first_match_column_page(&self) -> Option<usize> {
        self.highlighted_cells
            .first()
            .map(|&(_, column)| column / self.columns_per_page)
    }

    pub(super) fn preview_height(&self) -> f32 {
        2. + ROW_HEIGHT * self.rows.len().min(MAX_DATA_ROWS + 1) as f32
    }

    pub(super) fn row_pages(&self) -> usize {
        self.data_rows.div_ceil(MAX_DATA_ROWS)
    }

    pub(super) fn column_pages(&self) -> usize {
        self.columns.div_ceil(self.columns_per_page)
    }

    pub(super) fn page_row_position(&self, page: usize) -> Option<(usize, usize)> {
        let pages = self.row_pages();
        (pages > 1 && page < pages).then(|| (1 + page * MAX_DATA_ROWS, self.data_rows))
    }

    pub(super) fn page_for_data_row(&self, number: usize) -> Option<usize> {
        (1..=self.data_rows)
            .contains(&number)
            .then(|| (number - 1) / MAX_DATA_ROWS)
    }

    pub(super) fn page_column_position(&self, page: usize) -> Option<(usize, usize)> {
        let pages = self.column_pages();
        (pages > 1 && page < pages).then(|| (self.visible_columns(page).start + 1, self.columns))
    }

    pub(super) fn page_for_column(&self, number: usize) -> Option<usize> {
        (1..=self.columns)
            .contains(&number)
            .then(|| (number - 1) / self.columns_per_page)
    }

    pub(super) fn page_first_data_cell(
        &self,
        row_page: usize,
        column_page: usize,
    ) -> Option<(usize, usize)> {
        if row_page >= self.row_pages() || column_page >= self.column_pages() {
            return None;
        }
        let row = 1 + row_page * MAX_DATA_ROWS;
        let column = 1 + self.visible_columns(column_page).start;
        Some((row, column))
    }

    pub(super) fn spans_pages(&self) -> bool {
        self.row_pages() > 1 || self.column_pages() > 1
    }

    pub(super) fn page_csv(&self, row_page: usize, column_page: usize) -> String {
        write_rows(self.visible_rows(row_page, column_page), ',')
    }

    pub(super) fn page_tsv(&self, row_page: usize, column_page: usize) -> String {
        write_rows(self.visible_rows(row_page, column_page), '\t')
    }

    pub(super) fn tsv(&self) -> String {
        write_rows(&self.rows, '\t')
    }

    pub(super) fn legend(&self) -> Div {
        div()
            .min_w_0()
            .truncate()
            .child(format!("{} 行 · {} 列", self.data_rows, self.columns))
    }

    fn visible_columns(&self, page: usize) -> std::ops::Range<usize> {
        let first = page * self.columns_per_page;
        first..self.columns.min(first + self.columns_per_page)
    }

    fn visible_rows(&self, row_page: usize, column_page: usize) -> impl Iterator<Item = &[String]> {
        let first_record = 1 + row_page * MAX_DATA_ROWS;
        let columns = self.visible_columns(column_page);
        self.rows
            .first()
            .into_iter()
            .chain(self.rows.iter().skip(first_record).take(MAX_DATA_ROWS))
            .map(move |row| &row[columns.clone()])
    }

    fn column_width_fractions(&self, column_page: usize) -> Vec<f32> {
        let widths = &self.column_width_units[self.visible_columns(column_page)];
        if widths.is_empty() {
            return Vec::new();
        }
        let total = widths.iter().sum::<usize>() as f32;
        let equal_share = 0.5 / widths.len() as f32;
        widths
            .iter()
            .map(|&width| equal_share + 0.5 * width as f32 / total)
            .collect()
    }

    pub(super) fn render(&self, row_page: usize, column_page: usize) -> Div {
        let rows = self.visible_rows(row_page, column_page);
        let column_widths = self.column_width_fractions(column_page);
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
                let first_column = column_page * self.columns_per_page;
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
                            .w(relative(column_widths[column]))
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

fn cell_width_units(value: &str) -> usize {
    let characters = value
        .chars()
        .take(MAX_COLUMN_WIDTH_UNITS)
        .map(|character| 1 + usize::from(!character.is_ascii() || character.is_ascii_control()))
        .sum::<usize>();
    (characters + 2).clamp(MIN_COLUMN_WIDTH_UNITS, MAX_COLUMN_WIDTH_UNITS)
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
    fn gives_long_table_cells_more_room_while_preserving_every_visible_column() {
        let rows = [
            ["数据行", "列号", "列名", "规则号", "规则", "内容", "备注"],
            ["2", "1", "日期", "1", "日期", "2023-02-29", "文字"],
            ["2", "3", "比率", "3", "数值范围", "1.01", "新值"],
        ];
        let table = TablePreview::from_rows(
            rows.into_iter()
                .map(|row| row.into_iter().map(str::to_owned).collect())
                .collect(),
        );
        let widths = table.column_width_fractions(0);
        assert_eq!(widths.len(), 6);
        assert!(widths[5] > 1. / 6.);
        assert!(widths.iter().all(|width| *width >= 0.1));
        assert!((widths.iter().sum::<f32>() - 1.).abs() < 1e-6);
        assert_eq!(table.column_width_fractions(1), vec![1.]);
        assert_eq!(cell_width_units(&"长".repeat(1000)), MAX_COLUMN_WIDTH_UNITS);
    }

    #[test]
    fn exports_visible_rows_and_columns_with_the_header_and_full_cell_values() {
        let mut rows = vec![["id", "label", "name", "note"]];
        rows.extend([["1", "初页", "甲", "原始"]].repeat(6));
        rows.push(["7", "末页", "绿茶,冰", " 原样\n\"备注\" "]);
        rows.push(["8", "末页", "", "  "]);
        let preview = TablePreview::from_rows(
            rows.into_iter()
                .map(|row| row.into_iter().map(str::to_owned).collect())
                .collect(),
        )
        .with_columns_per_page(2);

        assert_eq!(
            preview.page_csv(1, 1),
            "name,note\r\n\"绿茶,冰\",\" 原样\n\"\"备注\"\" \"\r\n\"\",  "
        );
        assert_eq!(preview.page_csv(1, 0), "id,label\r\n7,末页\r\n8,末页");
        assert_eq!(
            preview.page_tsv(1, 1),
            "name\tnote\r\n绿茶,冰\t\" 原样\n\"\"备注\"\" \"\r\n\"\"\t  "
        );
    }

    #[test]
    fn serializes_all_rows_and_columns_as_tsv_independently_of_paging() {
        let rows = [
            ["编号", "备注", "值"],
            ["001", "甲,乙", "1/3"],
            ["002", "含\t制表符", ""],
            ["003", "带\n\"换行\"", "0.50"],
            ["004", "", "4"],
            ["005", "  原样  ", "5"],
            ["006", "", "6"],
            ["007", "末行", "7"],
        ]
        .into_iter()
        .map(|row| row.map(str::to_owned).into())
        .collect();
        let preview = TablePreview::from_rows(rows).with_columns_per_page(2);
        assert_eq!((preview.row_pages(), preview.column_pages()), (2, 2));
        assert_eq!(
            preview.tsv(),
            "编号\t备注\t值\r\n001\t甲,乙\t1/3\r\n002\t\"含\t制表符\"\t\"\"\r\n003\t\"带\n\"\"换行\"\"\"\t0.50\r\n004\t\"\"\t4\r\n005\t  原样  \t5\r\n006\t\"\"\t6\r\n007\t末行\t7"
        );
    }

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
