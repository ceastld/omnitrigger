//! 鼠标手势的形状模板与相似度匹配。
//!
//! 一个手势是一条带方向的轨迹：沿路径等距取 [`GESTURE_SHAPE_POINTS`] 个点，平移到外包框中心并
//! 等比缩放到固定大小，得到与位置、大小无关的形状模板（不做旋转归一化，方向不同就是不同的
//! 手势）。松开时把所画轨迹化为同样的形状，与每个候选模板逐点比较距离得到 0–1000 的相似度，
//! 由引擎取最高分；分数接近的候选交给界面让用户选择。
//!
//! 全部运算为整数，结果只取决于输入坐标；轨迹缓冲为固定容量，移动时只做有界工作量，
//! 可在 Hook 同步路径中调用，也可供界面录制复用同一规则。

use std::fmt;

/// 从按下点位移达到此距离（物理像素）后进入手势，不再回放原按压。
pub const GESTURE_START_PX: u32 = 12;
/// 按下后严格小于此时长内未达到起始位移时，放弃手势并交还原按压。
pub const GESTURE_START_MS: u32 = 500;
/// 形状模板的采样点数。
pub const GESTURE_SHAPE_POINTS: usize = 32;
/// 轨迹总长（物理像素）不足此值时不构成手势。
pub const GESTURE_MIN_LENGTH_PX: u32 = 32;
/// 相似度（千分比）达到此值才算匹配。
pub const GESTURE_ACCEPT: u16 = 700;
/// 与最高分相差不超过此值的其他候选视为难以区分，需要用户选择。
pub const GESTURE_CLOSE_MARGIN: u16 = 60;
/// 两个模板的相似度达到此值视为同一个手势，不能在重叠范围内重复注册。
pub const GESTURE_DUPLICATE: u16 = 900;
/// 一次选择最多列出的候选数。
pub const MAX_GESTURE_CHOICES: usize = 4;

/// 归一化坐标的半边长：较长一边恰好占满 `-EXTENT..=EXTENT`。
const EXTENT: i64 = 1000;
/// 重采样时的定点倍率，避免短线段的长度被整数开方抹平。
const FIXED: i64 = 256;
/// 轨迹缓冲容量；写满时隔点抽稀并把采样间距加倍。
const TRACE_POINTS: usize = 128;
/// 轨迹缓冲的初始采样间距（物理像素）。
const TRACE_SPACING_PX: i64 = 3;
/// 单个坐标差的限幅：远大于任何屏幕，又保证平方与乘积不溢出。
const DELTA_LIMIT: i64 = 1 << 20;

/// 屏幕坐标中的方向，y 轴向下；用于以方向序列简写折线形状。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum GestureDirection {
    Up,
    Down,
    Left,
    Right,
    UpLeft,
    UpRight,
    DownLeft,
    DownRight,
}

impl GestureDirection {
    pub const ALL: [Self; 8] = [
        Self::Up,
        Self::Down,
        Self::Left,
        Self::Right,
        Self::UpLeft,
        Self::UpRight,
        Self::DownLeft,
        Self::DownRight,
    ];

    /// 配置文件中使用的稳定名称。
    pub fn token(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
            Self::Left => "left",
            Self::Right => "right",
            Self::UpLeft => "up_left",
            Self::UpRight => "up_right",
            Self::DownLeft => "down_left",
            Self::DownRight => "down_right",
        }
    }

    pub fn from_token(token: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|direction| direction.token() == token)
    }

    fn vector(self) -> (i32, i32) {
        match self {
            Self::Up => (0, -1),
            Self::Down => (0, 1),
            Self::Left => (-1, 0),
            Self::Right => (1, 0),
            Self::UpLeft => (-1, -1),
            Self::UpRight => (1, -1),
            Self::DownLeft => (-1, 1),
            Self::DownRight => (1, 1),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GestureShapeError {
    /// 少于两个点，或所有点重合。
    Empty,
    /// 轨迹总长不足 [`GESTURE_MIN_LENGTH_PX`]。
    TooShort,
    /// 方向序列中相邻两步相同；连续同向只是更长的一笔。
    RepeatedStep { index: usize },
}

impl fmt::Display for GestureShapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("手势至少需要两个不重合的点。"),
            Self::TooShort => write!(
                f,
                "手势太短，请至少画出 {GESTURE_MIN_LENGTH_PX} 像素的轨迹。"
            ),
            Self::RepeatedStep { index } => write!(
                f,
                "手势第 {} 步与上一步方向相同；连续同向只是更长的一笔，请删除重复方向。",
                index + 1
            ),
        }
    }
}

impl std::error::Error for GestureShapeError {}

/// 归一化的手势形状：沿路径等距的 [`GESTURE_SHAPE_POINTS`] 个点，较长一边占满 ±1000。
#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub struct GestureShape {
    points: [(i16, i16); GESTURE_SHAPE_POINTS],
}

impl fmt::Debug for GestureShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (first, last) = (self.points[0], self.points[GESTURE_SHAPE_POINTS - 1]);
        write!(f, "GestureShape({first:?}..{last:?})")
    }
}

impl GestureShape {
    /// 由任意折线（至少两个不重合的点，任意坐标与点数）得到形状：等距重采样后归一化。
    pub fn from_polyline(points: &[(i32, i32)]) -> Result<Self, GestureShapeError> {
        Self::normalize(&resample(points).ok_or(GestureShapeError::Empty)?)
    }

    /// 由已有的采样点得到形状。恰好 [`GESTURE_SHAPE_POINTS`] 个点时不再重采样，
    /// 只做平移与缩放；对已归一化的形状是恒等变换，保存后再载入不会漂移。
    pub fn from_points(points: &[(i32, i32)]) -> Result<Self, GestureShapeError> {
        match <[(i32, i32); GESTURE_SHAPE_POINTS]>::try_from(points) {
            Ok(exact) => Self::normalize(&exact.map(|(x, y)| (i64::from(x), i64::from(y)))),
            Err(_) => Self::from_polyline(points),
        }
    }

    /// 由方向序列生成等长笔画的折线形状；相邻方向相同返回错误，不合并。
    pub fn from_directions(steps: &[GestureDirection]) -> Result<Self, GestureShapeError> {
        if let Some(index) = steps.windows(2).position(|pair| pair[0] == pair[1]) {
            return Err(GestureShapeError::RepeatedStep { index: index + 1 });
        }
        const STEP: i32 = 1000;
        const DIAGONAL: i32 = 707;
        let mut points = Vec::with_capacity(steps.len() + 1);
        let mut position = (0, 0);
        points.push(position);
        for step in steps {
            let (dx, dy) = step.vector();
            let unit = if dx != 0 && dy != 0 { DIAGONAL } else { STEP };
            position = (position.0 + dx * unit, position.1 + dy * unit);
            points.push(position);
        }
        Self::from_polyline(&points)
    }

    pub fn points(&self) -> &[(i16, i16); GESTURE_SHAPE_POINTS] {
        &self.points
    }

    /// 与另一形状的相似度（千分比）：对应点平均距离为 0 时是 1000，
    /// 达到归一化半边长（方向大致垂直）时降到 0。
    pub fn similarity(&self, other: &Self) -> u16 {
        let total: u64 = self
            .points
            .iter()
            .zip(&other.points)
            .map(|(a, b)| {
                let dx = i64::from(a.0) - i64::from(b.0);
                let dy = i64::from(a.1) - i64::from(b.1);
                ((dx * dx + dy * dy) as u64).isqrt()
            })
            .sum();
        let average = total / GESTURE_SHAPE_POINTS as u64;
        (EXTENT as u64).saturating_sub(average) as u16
    }

    fn normalize(points: &[(i64, i64); GESTURE_SHAPE_POINTS]) -> Result<Self, GestureShapeError> {
        let (mut left, mut top, mut right, mut bottom) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
        for &(x, y) in points {
            left = left.min(x);
            top = top.min(y);
            right = right.max(x);
            bottom = bottom.max(y);
        }
        let span = (right - left).max(bottom - top);
        if span <= 0 {
            return Err(GestureShapeError::Empty);
        }
        // 以外包框中心为原点、较长一边为 2×EXTENT；四舍五入到整数坐标。
        let scale = |value: i64, low: i64, high: i64| {
            let doubled = (2 * value - low - high) * EXTENT;
            let rounded = if doubled >= 0 {
                (doubled + span / 2) / span
            } else {
                (doubled - span / 2) / span
            };
            rounded.clamp(-EXTENT, EXTENT) as i16
        };
        Ok(Self {
            points: points.map(|(x, y)| (scale(x, left, right), scale(y, top, bottom))),
        })
    }
}

fn clamp_delta(value: i64) -> i64 {
    value.clamp(-DELTA_LIMIT, DELTA_LIMIT)
}

/// 线段长度（定点）；坐标差先限幅，平方不会溢出。
fn segment_length(from: (i64, i64), to: (i64, i64)) -> i64 {
    let dx = clamp_delta(to.0 - from.0) * FIXED;
    let dy = clamp_delta(to.1 - from.1) * FIXED;
    ((dx * dx + dy * dy) as u64).isqrt() as i64
}

/// 沿折线按路径长度等距取 [`GESTURE_SHAPE_POINTS`] 个点（定点坐标）；总长为零时返回 `None`。
fn resample(points: &[(i32, i32)]) -> Option<[(i64, i64); GESTURE_SHAPE_POINTS]> {
    let at = |index: usize| (i64::from(points[index].0), i64::from(points[index].1));
    let total: i64 = (1..points.len())
        .map(|index| segment_length(at(index - 1), at(index)))
        .sum();
    if total <= 0 {
        return None;
    }
    let mut output = [(0, 0); GESTURE_SHAPE_POINTS];
    let (mut index, mut walked) = (1, 0i64);
    for (slot, sample) in output.iter_mut().enumerate() {
        let target = total * slot as i64 / (GESTURE_SHAPE_POINTS as i64 - 1);
        // 前进到包含目标长度的线段；零长线段直接跳过。
        let mut length = segment_length(at(index - 1), at(index));
        while index + 1 < points.len() && walked + length < target {
            walked += length;
            index += 1;
            length = segment_length(at(index - 1), at(index));
        }
        let (from, to) = (at(index - 1), at(index));
        let along = (target - walked).clamp(0, length);
        let lerp = |a: i64, b: i64| {
            if length == 0 {
                b * FIXED
            } else {
                a * FIXED + clamp_delta(b - a) * FIXED * along / length
            }
        };
        *sample = (lerp(from.0, to.0), lerp(from.1, to.1));
    }
    Some(output)
}

/// 固定容量的轨迹记录器；只接收显式提供的物理坐标。
#[derive(Clone, Debug)]
pub struct GestureTrace {
    origin: (i32, i32),
    tip: (i32, i32),
    started: bool,
    points: [(i32, i32); TRACE_POINTS],
    len: usize,
    spacing: i64,
}

impl GestureTrace {
    pub fn new(origin: (i32, i32)) -> Self {
        let mut points = [(0, 0); TRACE_POINTS];
        points[0] = origin;
        Self {
            origin,
            tip: origin,
            started: false,
            points,
            len: 1,
            spacing: TRACE_SPACING_PX,
        }
    }

    pub fn origin(&self) -> (i32, i32) {
        self.origin
    }

    /// 是否曾离开按下点达到 [`GESTURE_START_PX`]。
    pub fn started(&self) -> bool {
        self.started
    }

    pub fn push(&mut self, position: (i32, i32)) {
        let delta = |from: (i32, i32)| {
            (
                i64::from(position.0) - i64::from(from.0),
                i64::from(position.1) - i64::from(from.1),
            )
        };
        self.tip = position;
        if !self.started {
            let (dx, dy) = delta(self.origin);
            let start = i64::from(GESTURE_START_PX);
            // 各轴先与阈值比较，只有较小差值才平方，极端坐标不会溢出。
            self.started =
                dx.abs() >= start || dy.abs() >= start || dx * dx + dy * dy >= start * start;
        }
        let (dx, dy) = delta(self.points[self.len - 1]);
        if dx.abs() < self.spacing && dy.abs() < self.spacing {
            return;
        }
        if self.len == TRACE_POINTS {
            // 隔点抽稀并把间距加倍：形状保持，长轨迹的工作量与内存仍有界。
            for index in 1..TRACE_POINTS / 2 {
                self.points[index] = self.points[index * 2];
            }
            self.len = TRACE_POINTS / 2;
            self.spacing = self.spacing.saturating_mul(2);
        }
        self.points[self.len] = position;
        self.len += 1;
    }

    /// 当前轨迹的形状；总长不足 [`GESTURE_MIN_LENGTH_PX`] 时返回错误。
    pub fn shape(&self) -> Result<GestureShape, GestureShapeError> {
        let mut points = [(0, 0); TRACE_POINTS + 1];
        points[..self.len].copy_from_slice(&self.points[..self.len]);
        points[self.len] = self.tip;
        let points = &points[..=self.len];
        let at = |index: usize| (i64::from(points[index].0), i64::from(points[index].1));
        let length: i64 = (1..points.len())
            .map(|index| segment_length(at(index - 1), at(index)))
            .sum();
        if length < i64::from(GESTURE_MIN_LENGTH_PX) * FIXED {
            return Err(GestureShapeError::TooShort);
        }
        GestureShape::from_polyline(points)
    }
}

/// 松开时难以区分的候选：按相似度从高到低，至少两项。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GestureChoice {
    ids: [u64; MAX_GESTURE_CHOICES],
    scores: [u16; MAX_GESTURE_CHOICES],
    len: u8,
}

impl GestureChoice {
    pub(crate) fn new(candidates: &[(u64, u16)]) -> Self {
        let mut choice = Self {
            ids: [0; MAX_GESTURE_CHOICES],
            scores: [0; MAX_GESTURE_CHOICES],
            len: candidates.len().min(MAX_GESTURE_CHOICES) as u8,
        };
        for (slot, &(id, score)) in candidates.iter().take(MAX_GESTURE_CHOICES).enumerate() {
            choice.ids[slot] = id;
            choice.scores[slot] = score;
        }
        choice
    }

    /// 候选的绑定标识与相似度（千分比）。
    pub fn candidates(&self) -> impl Iterator<Item = (u64, u16)> + '_ {
        let len = usize::from(self.len);
        self.ids[..len]
            .iter()
            .copied()
            .zip(self.scores[..len].iter().copied())
    }
}
