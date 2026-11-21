use std::borrow::Cow;

use unicode_segmentation::UnicodeSegmentation;

pub(super) const COLUMNS: usize = 40;
const TAB_SPACING: &str = "  ";

pub(super) fn column_width(grapheme: &str) -> usize {
    match grapheme {
        "\t" => TAB_SPACING.len(),
        _ if grapheme.is_ascii() => 1,
        _ => 2,
    }
}

pub(super) fn display_tabs(text: &str) -> Cow<'_, str> {
    if text.contains('\t') {
        Cow::Owned(text.replace('\t', TAB_SPACING))
    } else {
        Cow::Borrowed(text)
    }
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

fn positioned_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut offset = 0;
    text.split('\n').flat_map(move |line| {
        let start = offset;
        offset += line.len() + 1;
        let mut pending = Some((start, line.strip_suffix('\r').unwrap_or(line)));
        std::iter::from_fn(move || {
            let (start, line) = pending.take()?;
            let mut columns = 0;
            let overflow = line.grapheme_indices(true).find_map(|(index, grapheme)| {
                columns += column_width(grapheme);
                (columns > COLUMNS).then_some(index)
            });
            if let Some(index) = overflow {
                let (first, rest) = line.split_at(index);
                pending = Some((start + index, rest));
                Some((start, first))
            } else {
                Some((start, line))
            }
        })
    })
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
    fn tabs_are_visible_in_results_and_wrap_at_their_display_width() {
        assert_eq!(display_tabs("2\t苹果"), "2  苹果");
        let prefix = "x".repeat(39);
        let text = format!("{prefix}\ta");
        assert_eq!(lines(&text).collect::<Vec<_>>(), [prefix.as_str(), "\ta"]);
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
