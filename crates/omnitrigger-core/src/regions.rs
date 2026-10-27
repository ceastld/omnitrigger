//! 可重放的物理屏幕几何；平台只提供矩形，不在这里读取显示器或鼠标。
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScreenRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl ScreenRect {
    pub fn is_valid(self) -> bool {
        self.left < self.right && self.top < self.bottom
    }

    pub fn contains(self, point: (i32, i32)) -> bool {
        self.contains_wide(i64::from(point.0), i64::from(point.1))
    }

    fn contains_wide(self, x: i64, y: i64) -> bool {
        x >= i64::from(self.left)
            && x < i64::from(self.right)
            && y >= i64::from(self.top)
            && y < i64::from(self.bottom)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScreenEdge {
    Top,
    Right,
    Bottom,
    Left,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScreenCorner {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PointerRegion {
    ScreenEdge { edge: ScreenEdge, thickness_px: u32 },
    ScreenCorner { corner: ScreenCorner, size_px: u32 },
    Rectangle(ScreenRect),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TriggerCondition {
    PointerRegion(PointerRegion),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegionError {
    InvalidThickness { thickness_px: u32 },
    InvalidCornerSize { size_px: u32 },
    InvalidRectangle { rect: ScreenRect },
}

impl fmt::Display for RegionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidThickness { .. } => "屏幕边缘宽度需要 1 至 256 个物理像素。",
            Self::InvalidCornerSize { .. } => "屏幕角落大小需要 1 至 1024 个物理像素。",
            Self::InvalidRectangle { .. } => "指定区域必须是宽高均大于零的半开矩形。",
        })
    }
}
impl std::error::Error for RegionError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContextError {
    TooManyScreens { count: usize },
    InvalidScreen { index: usize, rect: ScreenRect },
}
impl fmt::Display for ContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooManyScreens { .. } => "屏幕上下文最多包含 32 个显示器。",
            Self::InvalidScreen { .. } => "显示器上下文包含宽高无效的矩形。",
        })
    }
}
impl std::error::Error for ContextError {}

impl PointerRegion {
    pub fn validate(self) -> Result<(), RegionError> {
        match self {
            Self::ScreenEdge { thickness_px, .. } if !(1..=256).contains(&thickness_px) => {
                Err(RegionError::InvalidThickness { thickness_px })
            }
            Self::ScreenCorner { size_px, .. } if !(1..=1024).contains(&size_px) => {
                Err(RegionError::InvalidCornerSize { size_px })
            }
            Self::Rectangle(rect) if !rect.is_valid() => {
                Err(RegionError::InvalidRectangle { rect })
            }
            _ => Ok(()),
        }
    }

    pub fn matches(self, point: (i32, i32), screens: &[ScreenRect]) -> bool {
        if self.validate().is_err() {
            return false;
        }
        if let Self::Rectangle(rect) = self {
            return rect.contains(point);
        }
        screens
            .iter()
            .copied()
            .filter(|screen| screen.is_valid() && screen.contains(point))
            .any(|screen| match self {
                Self::ScreenEdge { edge, thickness_px } => {
                    edge_distance(screen, point, edge) < i64::from(thickness_px)
                        && owns_half(screen, point, edge)
                        && exposed(screen, point, edge, screens)
                }
                Self::ScreenCorner { corner, size_px } => {
                    let (horizontal, vertical) = corner_edges(corner);
                    let anchor = corner_anchor(screen, corner);
                    corner_diagonal_exposed(screen, corner, screens)
                        && [horizontal, vertical].into_iter().all(|edge| {
                            edge_distance(screen, point, edge) < i64::from(size_px)
                                && owns_half(screen, point, edge)
                                && exposed(screen, anchor, edge, screens)
                                && exposed(screen, point, edge, screens)
                        })
                }
                Self::Rectangle(_) => false,
            })
    }
}

impl TriggerCondition {
    pub fn matches(self, point: Option<(i32, i32)>, screens: &[ScreenRect]) -> bool {
        match self {
            Self::PointerRegion(region) => {
                point.is_some_and(|point| region.matches(point, screens))
            }
        }
    }
}

fn edge_distance(screen: ScreenRect, point: (i32, i32), edge: ScreenEdge) -> i64 {
    match edge {
        ScreenEdge::Left => i64::from(point.0) - i64::from(screen.left),
        ScreenEdge::Right => i64::from(screen.right) - 1 - i64::from(point.0),
        ScreenEdge::Top => i64::from(point.1) - i64::from(screen.top),
        ScreenEdge::Bottom => i64::from(screen.bottom) - 1 - i64::from(point.1),
    }
}

// 参数是最大范围；相反边争用窄屏中心像素时，固定归左/上，不依赖绑定顺序。
fn owns_half(screen: ScreenRect, point: (i32, i32), edge: ScreenEdge) -> bool {
    let distance = edge_distance(screen, point, edge);
    let opposite = edge_distance(
        screen,
        point,
        match edge {
            ScreenEdge::Left => ScreenEdge::Right,
            ScreenEdge::Right => ScreenEdge::Left,
            ScreenEdge::Top => ScreenEdge::Bottom,
            ScreenEdge::Bottom => ScreenEdge::Top,
        },
    );
    match edge {
        ScreenEdge::Left | ScreenEdge::Top => distance <= opposite,
        ScreenEdge::Right | ScreenEdge::Bottom => distance < opposite,
    }
}

fn exposed(
    screen: ScreenRect,
    point: (i32, i32),
    edge: ScreenEdge,
    screens: &[ScreenRect],
) -> bool {
    let (x, y) = match edge {
        ScreenEdge::Left => (i64::from(screen.left) - 1, i64::from(point.1)),
        ScreenEdge::Right => (i64::from(screen.right), i64::from(point.1)),
        ScreenEdge::Top => (i64::from(point.0), i64::from(screen.top) - 1),
        ScreenEdge::Bottom => (i64::from(point.0), i64::from(screen.bottom)),
    };
    !screens
        .iter()
        .copied()
        .any(|other| other.is_valid() && other.contains_wide(x, y))
}

fn corner_edges(corner: ScreenCorner) -> (ScreenEdge, ScreenEdge) {
    match corner {
        ScreenCorner::TopLeft => (ScreenEdge::Left, ScreenEdge::Top),
        ScreenCorner::TopRight => (ScreenEdge::Right, ScreenEdge::Top),
        ScreenCorner::BottomRight => (ScreenEdge::Right, ScreenEdge::Bottom),
        ScreenCorner::BottomLeft => (ScreenEdge::Left, ScreenEdge::Bottom),
    }
}

fn corner_anchor(screen: ScreenRect, corner: ScreenCorner) -> (i32, i32) {
    match corner {
        ScreenCorner::TopLeft => (screen.left, screen.top),
        ScreenCorner::TopRight => (screen.right - 1, screen.top),
        ScreenCorner::BottomRight => (screen.right - 1, screen.bottom - 1),
        ScreenCorner::BottomLeft => (screen.left, screen.bottom - 1),
    }
}

fn corner_diagonal_exposed(
    screen: ScreenRect,
    corner: ScreenCorner,
    screens: &[ScreenRect],
) -> bool {
    let (x, y) = match corner {
        ScreenCorner::TopLeft => (i64::from(screen.left) - 1, i64::from(screen.top) - 1),
        ScreenCorner::TopRight => (i64::from(screen.right), i64::from(screen.top) - 1),
        ScreenCorner::BottomRight => (i64::from(screen.right), i64::from(screen.bottom)),
        ScreenCorner::BottomLeft => (i64::from(screen.left) - 1, i64::from(screen.bottom)),
    };
    !screens
        .iter()
        .copied()
        .any(|other| other.is_valid() && other.contains_wide(x, y))
}

pub(crate) fn conditions_overlap(left: &[TriggerCondition], right: &[TriggerCondition]) -> bool {
    let (Some(TriggerCondition::PointerRegion(left)), Some(TriggerCondition::PointerRegion(right))) =
        (left.first(), right.first())
    else {
        return true;
    };
    match (*left, *right) {
        (PointerRegion::Rectangle(left), PointerRegion::Rectangle(right)) => {
            left.left < right.right
                && right.left < left.right
                && left.top < right.bottom
                && right.top < left.bottom
        }
        (PointerRegion::Rectangle(_), _) | (_, PointerRegion::Rectangle(_)) => true,
        (
            PointerRegion::ScreenEdge { edge: left, .. },
            PointerRegion::ScreenEdge { edge: right, .. },
        ) => !matches!(
            (left, right),
            (ScreenEdge::Left, ScreenEdge::Right)
                | (ScreenEdge::Right, ScreenEdge::Left)
                | (ScreenEdge::Top, ScreenEdge::Bottom)
                | (ScreenEdge::Bottom, ScreenEdge::Top)
        ),
        (
            PointerRegion::ScreenCorner { corner: left, .. },
            PointerRegion::ScreenCorner { corner: right, .. },
        ) => left == right,
        (PointerRegion::ScreenEdge { edge, .. }, PointerRegion::ScreenCorner { corner, .. })
        | (PointerRegion::ScreenCorner { corner, .. }, PointerRegion::ScreenEdge { edge, .. }) => {
            let (horizontal, vertical) = corner_edges(corner);
            edge == horizontal || edge == vertical
        }
    }
}

pub(crate) fn validate_screens(screens: &[ScreenRect]) -> Result<(), ContextError> {
    if screens.len() > 32 {
        return Err(ContextError::TooManyScreens {
            count: screens.len(),
        });
    }
    for (index, &rect) in screens.iter().enumerate() {
        if !rect.is_valid() {
            return Err(ContextError::InvalidScreen { index, rect });
        }
    }
    Ok(())
}
