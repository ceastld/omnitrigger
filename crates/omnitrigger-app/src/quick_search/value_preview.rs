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
    text.split('\n').flat_map(|line| {
        let mut pending = Some(line.strip_suffix('\r').unwrap_or(line));
        std::iter::from_fn(move || {
            let line = pending.take()?;
            let mut columns = 0;
            let overflow = line.grapheme_indices(true).find_map(|(index, grapheme)| {
                columns += column_width(grapheme);
                (columns > COLUMNS).then_some(index)
            });
            if let Some(index) = overflow {
                let (first, rest) = line.split_at(index);
                pending = Some(rest);
                Some(first)
            } else {
                Some(line)
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
}
