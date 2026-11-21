use unicode_segmentation::UnicodeSegmentation;

pub(super) const COLUMNS: usize = 40;

pub(super) fn column_width(grapheme: &str) -> usize {
    if grapheme.is_ascii() {
        1
    } else {
        2
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
}
