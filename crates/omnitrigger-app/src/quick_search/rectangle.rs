use omnitrigger_core::ScreenRect;

use super::aspect_ratio::pixel_dimensions;
use super::calculator::{normalize, QUERY_LIMIT};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RectangleError {
    Incomplete,
    InvalidCoordinates,
    InvalidSize,
    Empty,
    OutOfRange,
    TooLong,
}

impl RectangleError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入矩形的左、上、右、下，或输入左、上 @ 宽×高",
            Self::InvalidCoordinates => "坐标用逗号分隔的整数表示",
            Self::InvalidSize => "宽和高须是正整数，例如 100×200",
            Self::Empty => "左坐标须小于右坐标，上坐标须小于下坐标",
            Self::OutOfRange => "坐标超出可配置的物理像素范围",
            Self::TooLong => "矩形计算式过长",
        }
    }
}

pub(super) fn calculate_rectangle(query: &str) -> Result<String, RectangleError> {
    if query.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT {
        return Err(RectangleError::TooLong);
    }
    let input = normalize(query);
    let input = input.trim();
    if input.is_empty() {
        return Err(RectangleError::Incomplete);
    }
    if let Some((origin, size)) = input.split_once('@') {
        let [left, top] = coordinates(origin)?;
        let (width, height) = pixel_dimensions(size).map_err(|_| RectangleError::InvalidSize)?;
        let end = |start, extent| {
            i32::try_from(i64::from(start) + i64::from(extent))
                .map_err(|_| RectangleError::OutOfRange)
        };
        return Ok(rectangle_text(ScreenRect {
            left,
            top,
            right: end(left, width)?,
            bottom: end(top, height)?,
        }));
    }
    if let Some((region, offset)) = input.split_once("移动") {
        let rect = parse_rectangle(region)?;
        let [dx, dy] = coordinates(offset)?;
        let shifted = |coordinate: i32, delta| {
            coordinate
                .checked_add(delta)
                .ok_or(RectangleError::OutOfRange)
        };
        return Ok(rectangle_text(ScreenRect {
            left: shifted(rect.left, dx)?,
            top: shifted(rect.top, dy)?,
            right: shifted(rect.right, dx)?,
            bottom: shifted(rect.bottom, dy)?,
        }));
    }
    if let Some((left, right)) = input.split_once("交集") {
        let left = parse_rectangle(left)?;
        let right = parse_rectangle(right)?;
        let intersection = ScreenRect {
            left: left.left.max(right.left),
            top: left.top.max(right.top),
            right: left.right.min(right.right),
            bottom: left.bottom.min(right.bottom),
        };
        return Ok(if intersection.is_valid() {
            rectangle_text(intersection)
        } else {
            "无重叠".to_owned()
        });
    }
    let rect = parse_rectangle(input)?;
    let width = i64::from(rect.right) - i64::from(rect.left);
    let height = i64::from(rect.bottom) - i64::from(rect.top);
    let center_x = (f64::from(rect.left) + f64::from(rect.right)) / 2.;
    let center_y = (f64::from(rect.top) + f64::from(rect.bottom)) / 2.;
    Ok(format!(
        "{width} × {height} px · 中心 {center_x}, {center_y}"
    ))
}

fn coordinates<const COUNT: usize>(input: &str) -> Result<[i32; COUNT], RectangleError> {
    let mut fields = input.split(',');
    let mut values = [0; COUNT];
    for value in &mut values {
        let field = fields.next().ok_or(RectangleError::InvalidCoordinates)?;
        *value = field
            .trim()
            .parse()
            .map_err(|_| RectangleError::InvalidCoordinates)?;
    }
    if fields.next().is_some() {
        return Err(RectangleError::InvalidCoordinates);
    }
    Ok(values)
}

fn parse_rectangle(input: &str) -> Result<ScreenRect, RectangleError> {
    let [left, top, right, bottom] = coordinates(input)?;
    let rect = ScreenRect {
        left,
        top,
        right,
        bottom,
    };
    rect.is_valid().then_some(rect).ok_or(RectangleError::Empty)
}

fn rectangle_text(rect: ScreenRect) -> String {
    format!(
        "{}, {}, {}, {}",
        rect.left, rect.top, rect.right, rect.bottom
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepares_half_open_regions_accepted_by_trigger_conditions() {
        for (query, expected) in [
            ("−１９２０，５０＠１００×２００", "-1920, 50, -1820, 250"),
            ("-1920,50,-1820,250", "100 × 200 px · 中心 -1870, 150"),
            ("-2,-1,1,1", "3 × 2 px · 中心 -0.5, 0"),
            ("-1920,50,-1820,250 移动 50,-10", "-1870, 40, -1770, 240"),
            ("-20,0,20,30 交集 -10,-5,30,15", "-10, 0, 20, 15"),
            ("0,0,10,10 交集 10,0,20,10", "无重叠"),
            (
                "-2147483648,0 @ 4294967295×1",
                "-2147483648, 0, 2147483647, 1",
            ),
        ] {
            assert_eq!(
                calculate_rectangle(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for (query, expected) in [
            ("1,0,1,10", RectangleError::Empty),
            ("0,0 @ 0×1", RectangleError::InvalidSize),
            ("0,0 @ 1x2x3", RectangleError::InvalidSize),
            ("0,0,1,2,3", RectangleError::InvalidCoordinates),
            ("2147483647,0 @ 1x1", RectangleError::OutOfRange),
            (
                "2147483646,0,2147483647,1 移动 1,0",
                RectangleError::OutOfRange,
            ),
        ] {
            assert_eq!(calculate_rectangle(query), Err(expected), "{query}");
        }
    }
}
