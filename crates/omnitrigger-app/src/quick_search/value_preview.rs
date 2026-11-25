use std::borrow::Cow;

use unicode_segmentation::UnicodeSegmentation;

use super::text_tools::logical_newline_pattern;

pub(super) const COLUMNS: usize = 40;
const TAB_STOP: usize = 8;

fn column_width(grapheme: &str, column: usize) -> usize {
    match grapheme {
        "\t" => TAB_STOP - column % TAB_STOP,
        _ if grapheme.is_ascii() => 1,
        _ => 2,
    }
}

pub(super) fn columns<'a>(graphemes: impl Iterator<Item = &'a str>) -> usize {
    graphemes.fold(0, |column, grapheme| {
        column + column_width(grapheme, column)
    })
}

pub(super) fn display_tabs(text: &str) -> Cow<'_, str> {
    if !text.contains('\t') {
        return Cow::Borrowed(text);
    }
    let mut display = String::with_capacity(text.len());
    let mut column = 0;
    for grapheme in text.graphemes(true) {
        let width = column_width(grapheme, column);
        if grapheme == "\t" {
            display.extend(std::iter::repeat_n(' ', width));
        } else {
            display.push_str(grapheme);
        }
        column += width;
    }
    Cow::Owned(display)
}

pub(super) fn lines(text: &str) -> impl Iterator<Item = &str> {
    positioned_lines(text).map(|(_, line)| line)
}

pub(super) fn page_starts(text: &str, page_size: usize) -> Vec<usize> {
    positioned_lines(text)
        .step_by(page_size)
        .map(|(start, _)| start)
        .collect()
}

pub(super) fn page_text<'a>(text: &'a str, starts: &[usize], page: usize) -> Option<&'a str> {
    let start = *starts.get(page)?;
    let end = starts.get(page + 1).copied().unwrap_or(text.len());
    text.get(start..end)
}

fn positioned_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut line_start = 0;
    let line_ends = logical_newline_pattern()
        .find_iter(text)
        .map(|newline| (newline.start(), newline.end()))
        .chain(std::iter::once((text.len(), text.len())));
    line_ends.flat_map(move |(end, next_start)| {
        let mut pending = Some((line_start, &text[line_start..end]));
        line_start = next_start;
        std::iter::from_fn(move || {
            let (start, line) = pending.take()?;
            if let Some(index) = wrap_point(line) {
                let (first, rest) = line.split_at(index);
                pending = Some((start + index, rest));
                Some((start, first))
            } else {
                Some((start, line))
            }
        })
    })
}

fn wrap_point(line: &str) -> Option<usize> {
    let mut column = 0;
    let mut seen_text = false;
    let mut break_after = None;
    for (index, grapheme) in line.grapheme_indices(true) {
        let whitespace = matches!(grapheme, " " | "\t");
        let separator = whitespace || matches!(grapheme, "," | "，");
        column += column_width(grapheme, column);
        if column > COLUMNS {
            return Some(if whitespace {
                index
            } else {
                break_after.unwrap_or(index)
            });
        }
        if separator && seen_text && column >= COLUMNS / 2 {
            break_after = Some(index + grapheme.len());
        }
        seen_text |= !separator;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_keeps_line_breaks_and_wraps_whole_graphemes() {
        assert_eq!(
            lines("北京\r\n\r\n  上海\n").collect::<Vec<_>>(),
            ["北京", "", "  上海", ""]
        );
        let prefix = "x".repeat(39);
        let mixed = format!("{prefix}👩‍💻a");
        assert_eq!(lines(&mixed).collect::<Vec<_>>(), [prefix.as_str(), "👩‍💻a"]);
    }

    #[test]
    fn splits_all_logical_line_breaks_without_losing_preview_page_offsets() {
        let text = "甲\r\n乙\r\u{2028}丙\u{2029}";
        let expected = ["甲", "乙", "", "丙", ""];
        assert_eq!(lines(text).collect::<Vec<_>>(), expected);
        let starts = page_starts(text, 1);
        assert_eq!(starts, [0, 5, 9, 12, 18]);
        for (offset, line) in starts.into_iter().zip(expected) {
            assert_eq!(lines(&text[offset..]).next(), Some(line));
        }
    }

    #[test]
    fn prefers_word_and_field_boundaries_and_keeps_the_same_page_offsets() {
        let prefix = "1".repeat(38);
        let text = format!("{prefix} dB · -45°");
        assert_eq!(lines(&text).collect::<Vec<_>>(), [&text[..39], "dB · -45°"]);
        assert_eq!(page_starts(&text, 1), [0, 39]);
        assert_eq!(lines(&text[39..]).next(), Some("dB · -45°"));
        let fields = format!("{prefix},梨,茶");
        assert_eq!(lines(&fields).collect::<Vec<_>>(), [&fields[..39], "梨,茶"]);
        for prefix in ["  ", "100 Hz,"] {
            let long_word = format!("{prefix}{}", "x".repeat(COLUMNS));
            assert_eq!(lines(&long_word).next(), Some(&long_word[..COLUMNS]));
        }
    }

    #[test]
    fn tabs_advance_to_fixed_columns_and_wrap_at_their_display_width() {
        for (input, expected) in [
            ("x\ty", "x       y"),
            ("-2\t13/3", "-2      13/3"),
            ("3/2\t31/12", "3/2     31/12"),
            ("苹果\t2", "苹果    2"),
            ("12345678\tx", "12345678        x"),
        ] {
            assert_eq!(display_tabs(input), expected, "{input}");
            assert_eq!(
                columns(input.graphemes(true)),
                columns(expected.graphemes(true))
            );
        }

        let prefix = "x".repeat(40);
        let text = format!("{prefix}\ta");
        assert_eq!(lines(&text).collect::<Vec<_>>(), [prefix.as_str(), "\ta"]);

        let text = format!("{}\ta", "x".repeat(39));
        assert_eq!(lines(&text).collect::<Vec<_>>(), [&text[..40], "a"]);
    }

    #[test]
    fn copies_each_page_with_its_original_byte_boundaries() {
        let wide = "界".repeat(COLUMNS);
        let first = format!(" 甲\t\r\n{wide}\u{2028}");
        let text = format!("{first}\r尾\t ");
        let starts = page_starts(&text, 3);
        assert_eq!(page_text(&text, &starts, 0), Some(first.as_str()));
        assert_eq!(page_text(&text, &starts, 1), Some("\r尾\t "));
        assert_eq!(page_text(&text, &starts, 2), None);

        let wrapped = page_starts(&wide, 1);
        let first_half = "界".repeat(COLUMNS / 2);
        assert_eq!(page_text(&wide, &wrapped, 0), Some(first_half.as_str()));
        assert_eq!(page_text(&wide, &wrapped, 1), Some(first_half.as_str()));
    }

    #[test]
    fn page_offsets_keep_wrapped_graphemes_and_blank_lines_in_order() {
        let prefix = "x".repeat(39);
        let text = format!("甲\r\n{prefix}👩‍💻\t乙\r\n\r\n收尾\n");
        let expected = ["甲", prefix.as_str(), "👩‍💻\t乙", "", "收尾", ""];
        for page_size in [1, 2, 4, 12] {
            let starts = page_starts(&text, page_size);
            assert_eq!(starts.len(), expected.len().div_ceil(page_size));
            for (start, page) in starts.into_iter().zip(expected.chunks(page_size)) {
                assert_eq!(
                    lines(&text[start..]).take(page_size).collect::<Vec<_>>(),
                    page
                );
            }
        }
    }
}
