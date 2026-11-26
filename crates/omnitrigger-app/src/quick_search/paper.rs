use super::batch::batch_arguments;
use super::calculator::{evaluate, format_result, normalize, QUERY_LIMIT};
use super::conversion::{split_conversion, INCH_IN_METERS};

const SERIES_A: [(u32, u32); 11] = [
    (841, 1189),
    (594, 841),
    (420, 594),
    (297, 420),
    (210, 297),
    (148, 210),
    (105, 148),
    (74, 105),
    (52, 74),
    (37, 52),
    (26, 37),
];
const SERIES_B: [(u32, u32); 11] = [
    (1000, 1414),
    (707, 1000),
    (500, 707),
    (353, 500),
    (250, 353),
    (176, 250),
    (125, 176),
    (88, 125),
    (62, 88),
    (44, 62),
    (31, 44),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PaperError {
    Incomplete,
    UnknownSize,
    InvalidDpi,
    InvalidBleed,
    OutOfRange,
    TooLong,
}

impl PaperError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入纸型，例如 A4、A4 @ 300 dpi 或 A4 to A5",
            Self::UnknownSize => "可用 ISO A0 至 A10、B0 至 B10、2A0、4A0、Letter 或 Legal",
            Self::InvalidDpi => "输入大于零的分辨率，例如 300 dpi",
            Self::InvalidBleed => "填写纸型和非负出血量，例如 出血 A4 @ 3 mm",
            Self::OutOfRange => "像素尺寸超出可计算范围",
            Self::TooLong => "纸张计算式过长",
        }
    }
}

pub(super) fn calculate_paper(query: &str) -> Result<String, PaperError> {
    if query.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT {
        return Err(PaperError::TooLong);
    }
    let input = normalize(query).to_ascii_lowercase();
    let input = input.trim();
    if let Some(arguments) = batch_arguments(input, "出血", "bleed") {
        return paper_bleed(arguments);
    }
    if let Some((source, density)) = input.split_once(['@', '按']) {
        let (width, height) = paper_dimensions(source)?;
        return paper_pixels(f64::from(width), f64::from(height), density);
    }
    if let Some((source, target)) = split_conversion(input) {
        let (width, height) = paper_dimensions(source)?;
        let (target_width, target_height) = paper_dimensions(target)?;
        // 百分比保留两位并向下取整，确保复制出的缩放值仍能放入目标纸张。
        let scale = (target_width * 10_000 / width).min(target_height * 10_000 / height);
        return Ok(format!("{}.{:02}%", scale / 100, scale % 100));
    }
    let (width, height) = paper_dimensions(input)?;
    let mm = |length| f64::from(length) / 10.;
    Ok(format!("{} × {} mm", mm(width), mm(height)))
}

fn paper_bleed(input: &str) -> Result<String, PaperError> {
    let (paper, arguments) = input
        .split_once(['@', '按'])
        .ok_or(PaperError::InvalidBleed)?;
    let (width, height) = paper_dimensions(paper)?;
    let (margin, density) = arguments
        .split_once(['@', '按'])
        .map_or((arguments, None), |(margin, density)| {
            (margin, Some(density))
        });
    let margin = margin.trim();
    let margin = margin
        .strip_suffix("mm")
        .or_else(|| margin.strip_suffix("毫米"))
        .ok_or(PaperError::InvalidBleed)?;
    let margin = evaluate(margin).map_err(|_| PaperError::InvalidBleed)?;
    let added = margin * 20.;
    if margin < 0. || !added.is_finite() {
        return Err(PaperError::InvalidBleed);
    }
    let width = f64::from(width) + added;
    let height = f64::from(height) + added;
    if let Some(density) = density {
        return paper_pixels(width, height, density);
    }
    let width = format_result(width / 10.).map_err(|_| PaperError::InvalidBleed)?;
    let height = format_result(height / 10.).map_err(|_| PaperError::InvalidBleed)?;
    Ok(format!("{width} × {height} mm"))
}

fn paper_pixels(width: f64, height: f64, density: &str) -> Result<String, PaperError> {
    let density = density.trim();
    let density = density
        .strip_suffix("dpi")
        .or_else(|| density.strip_suffix("ppi"))
        .ok_or(PaperError::InvalidDpi)?;
    let density = evaluate(density).map_err(|_| PaperError::InvalidDpi)?;
    if density <= 0. {
        return Err(PaperError::InvalidDpi);
    }
    let pixels = |length: f64| {
        let count = (length / (INCH_IN_METERS * 10_000.) * density).round();
        if !(1. ..=f64::from(u32::MAX)).contains(&count) {
            return Err(PaperError::OutOfRange);
        }
        Ok(count as u32)
    };
    Ok(format!("{} × {} px", pixels(width)?, pixels(height)?))
}

// 以 0.1 mm 表示尺寸，以整数比较纸型的缩放比例。
fn paper_dimensions(input: &str) -> Result<(u32, u32), PaperError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(PaperError::Incomplete);
    }
    let landscape = input
        .strip_suffix("横向")
        .or_else(|| input.strip_suffix(" landscape"));
    let portrait = input
        .strip_suffix("纵向")
        .or_else(|| input.strip_suffix(" portrait"));
    let format = landscape.or(portrait).unwrap_or(input).trim_end();
    let format = format.strip_prefix("iso ").unwrap_or(format);
    let (width, height) = match format {
        "letter" | "us letter" => (2159, 2794),
        "legal" | "us legal" => (2159, 3556),
        "2a0" => (11890, 16820),
        "4a0" => (16820, 23780),
        _ => {
            let (series, index) = format.split_at_checked(1).ok_or(PaperError::UnknownSize)?;
            let sizes = match series {
                "a" => &SERIES_A,
                "b" => &SERIES_B,
                _ => return Err(PaperError::UnknownSize),
            };
            let index: usize = index.parse().map_err(|_| PaperError::UnknownSize)?;
            let &(width, height) = sizes.get(index).ok_or(PaperError::UnknownSize)?;
            (width * 10, height * 10)
        }
    };
    Ok(if landscape.is_some() {
        (height, width)
    } else {
        (width, height)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_equal_bleed_to_all_edges_in_millimeters_or_pixels() {
        for (query, expected) in [
            ("出血 A4 @ 3 mm", "216 × 303 mm"),
            ("出血 A4横向 按 3毫米 按 300 ppi", "3579 × 2551 px"),
            ("bleed letter landscape @ 1/2 mm", "280.4 × 216.9 mm"),
            ("出血 B5 @ 0 mm", "176 × 250 mm"),
        ] {
            assert_eq!(calculate_paper(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("出血 A4 @ -3 mm", PaperError::InvalidBleed),
            ("出血 A4 @ 3", PaperError::InvalidBleed),
            ("出血 A4 @ 3 mm @ 0 dpi", PaperError::InvalidDpi),
        ] {
            assert_eq!(calculate_paper(query), Err(error), "{query}");
        }
    }

    #[test]
    fn looks_up_paper_sizes_and_calculates_pixels_and_fitting_scale() {
        for (query, expected) in [
            ("A4", "210 × 297 mm"),
            ("ISO B5", "176 × 250 mm"),
            ("A10", "26 × 37 mm"),
            ("B10", "31 × 44 mm"),
            ("4A0", "1682 × 2378 mm"),
            ("US Letter", "215.9 × 279.4 mm"),
            ("legal landscape", "355.6 × 215.9 mm"),
            ("Ａ４＠３００ｄｐｉ", "2480 × 3508 px"),
            ("A4横向 按 300 ppi", "3508 × 2480 px"),
            ("Letter @ 300 dpi", "2550 × 3300 px"),
            ("A4 to A5", "70.47%"),
            ("A3 to A3", "100.00%"),
            ("Letter to A4", "97.26%"),
        ] {
            assert_eq!(calculate_paper(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("A11", PaperError::UnknownSize),
            ("210×297", PaperError::UnknownSize),
            ("A4 to", PaperError::Incomplete),
            ("A4 @ 0 dpi", PaperError::InvalidDpi),
            ("A4 @ 0.01 dpi", PaperError::OutOfRange),
            ("B0 @ 1e20 dpi", PaperError::OutOfRange),
        ] {
            assert_eq!(calculate_paper(query), Err(error), "{query}");
        }
    }
}
