//! 鼠标手势的方向序列与有界轨迹识别。
//!
//! 轨迹按固定段长采样并量化为方向，相同方向合并为一笔；已成形的一笔对落在扇区边界附近的
//! 采样段有粘性，贴着边界画的直线不会被切成交替的碎笔。短于下限的笔画视为抖动、转角或
//! 松开时的甩尾，识别结束时按“最短优先”删除并合并相邻同向笔画；下限取固定像素与最长一笔
//! 的固定比例中较大者，因此判定随手势大小缩放。识别器同时维护四向与八向两份读数：
//! 只含上下左右的手势与四向读数比较，含斜向的手势与八向读数比较。全部状态为固定容量，
//! 移动与读数都不分配内存，可在 Hook 同步路径中调用，也可供界面录制复用同一规则。

use std::fmt;

/// 一个手势最多包含的方向数。
pub const MAX_GESTURE_STEPS: usize = 8;
/// 从按下点位移达到此距离（物理像素）后进入手势，不再回放原按压。
pub const GESTURE_START_PX: u32 = 12;
/// 按下后严格小于此时长内未达到起始位移时，放弃手势并交还原按压。
pub const GESTURE_START_MS: u32 = 500;
/// 识别结果中每一笔的最小长度（物理像素）；更短的笔画按抖动或转角处理。
pub const GESTURE_MIN_STROKE_PX: u32 = 32;
/// 短于最长一笔此比例（1/N）的笔画同样按抖动处理，使判定随手势大小与屏幕密度缩放。
pub const GESTURE_STROKE_RATIO: u32 = 8;
/// 已成形的一笔保持方向的半角正切（分子，分母 10 000）：四向 55°，八向 30°。
/// 量化扇区半角分别为 45° 与 22.5°，多出的部分是边界两侧的滞回带。
const CARDINAL_STICKY_TAN: i64 = 14_281;
const OCTAL_STICKY_TAN: i64 = 5_774;
/// 采样段长：相对上一采样点位移达到此距离后量化一次方向。
const SEGMENT_PX: i64 = 16;
/// 原始笔画缓冲容量；写满时先删除已完成的短笔画，仍无法容纳则判定为过于复杂。
const RAW_STROKES: usize = 32;

/// 屏幕坐标中的方向，y 轴向下。
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

    pub fn is_diagonal(self) -> bool {
        matches!(
            self,
            Self::UpLeft | Self::UpRight | Self::DownLeft | Self::DownRight
        )
    }

    pub fn arrow(self) -> char {
        match self {
            Self::Up => '↑',
            Self::Down => '↓',
            Self::Left => '←',
            Self::Right => '→',
            Self::UpLeft => '↖',
            Self::UpRight => '↗',
            Self::DownLeft => '↙',
            Self::DownRight => '↘',
        }
    }

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

    /// 屏幕坐标中的方向向量；斜向未归一化，只用于比较夹角。
    fn vector(self) -> (i64, i64) {
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

    /// 位移与本方向的夹角正切是否不超过 `tan / 10 000`。位移已按 [`clamp_delta`] 限幅。
    fn holds(self, dx: i64, dy: i64, tan: i64) -> bool {
        let (ux, uy) = self.vector();
        let along = dx * ux + dy * uy;
        let across = (dx * uy - dy * ux).abs();
        along > 0 && across * 10_000 <= along * tan
    }

    /// 四向量化：水平与竖直等长时取水平。
    fn cardinal(dx: i64, dy: i64) -> Self {
        if dx.abs() >= dy.abs() {
            if dx >= 0 { Self::Right } else { Self::Left }
        } else if dy >= 0 {
            Self::Down
        } else {
            Self::Up
        }
    }

    /// 八向量化：以 22.5° 为界（tan 22.5° ≈ 0.4142），边界上取正方向。
    fn octal(dx: i64, dy: i64) -> Self {
        let (ax, ay) = (dx.abs(), dy.abs());
        if ay * 10_000 <= ax * 4_142 {
            return if dx >= 0 { Self::Right } else { Self::Left };
        }
        if ax * 10_000 <= ay * 4_142 {
            return if dy >= 0 { Self::Down } else { Self::Up };
        }
        match (dx >= 0, dy >= 0) {
            (true, true) => Self::DownRight,
            (true, false) => Self::UpRight,
            (false, true) => Self::DownLeft,
            (false, false) => Self::UpLeft,
        }
    }
}

/// 手势方向序列；相邻方向必须不同，最多 [`MAX_GESTURE_STEPS`] 步。
#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub struct GesturePath {
    steps: [GestureDirection; MAX_GESTURE_STEPS],
    len: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GesturePathError {
    Empty,
    TooLong { len: usize },
    RepeatedStep { index: usize },
}

impl fmt::Display for GesturePathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("手势至少需要一个方向。"),
            Self::TooLong { len } => {
                write!(f, "手势最多 {MAX_GESTURE_STEPS} 个方向，当前为 {len} 个。")
            }
            Self::RepeatedStep { index } => write!(
                f,
                "手势第 {} 步与上一步方向相同；连续同向会合并为一笔，请删除重复方向。",
                index + 1
            ),
        }
    }
}

impl std::error::Error for GesturePathError {}

impl GesturePath {
    /// 不合并或截断输入；重复方向和超长序列返回错误。
    pub fn new(steps: &[GestureDirection]) -> Result<Self, GesturePathError> {
        if steps.is_empty() {
            return Err(GesturePathError::Empty);
        }
        if steps.len() > MAX_GESTURE_STEPS {
            return Err(GesturePathError::TooLong { len: steps.len() });
        }
        if let Some(index) = steps.windows(2).position(|pair| pair[0] == pair[1]) {
            return Err(GesturePathError::RepeatedStep { index: index + 1 });
        }
        let mut path = Self {
            steps: [GestureDirection::Up; MAX_GESTURE_STEPS],
            len: steps.len() as u8,
        };
        path.steps[..steps.len()].copy_from_slice(steps);
        Ok(path)
    }

    pub fn steps(&self) -> &[GestureDirection] {
        &self.steps[..usize::from(self.len)]
    }

    pub fn has_diagonal(&self) -> bool {
        self.steps().iter().any(|step| step.is_diagonal())
    }
}

impl fmt::Display for GesturePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for step in self.steps() {
            write!(f, "{}", step.arrow())?;
        }
        Ok(())
    }
}

impl fmt::Debug for GesturePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GesturePath({self})")
    }
}

#[derive(Clone, Copy, Debug)]
struct Stroke {
    direction: GestureDirection,
    length: u32,
}

#[derive(Clone, Copy, Debug)]
struct StrokeLog {
    strokes: [Stroke; RAW_STROKES],
    len: usize,
    overflow: bool,
}

impl StrokeLog {
    const fn new() -> Self {
        Self {
            strokes: [Stroke {
                direction: GestureDirection::Up,
                length: 0,
            }; RAW_STROKES],
            len: 0,
            overflow: false,
        }
    }

    /// 采样段的方向：最后一笔已成形且位移仍在其滞回带内时沿用该笔方向，否则取量化结果。
    /// 未成形的短笔没有粘性，抖动不会借此变长。
    fn settle(&self, raw: GestureDirection, dx: i64, dy: i64, sticky_tan: i64) -> GestureDirection {
        match self.len.checked_sub(1).map(|index| self.strokes[index]) {
            Some(last)
                if last.length >= GESTURE_MIN_STROKE_PX
                    && last.direction.holds(dx, dy, sticky_tan) =>
            {
                last.direction
            }
            _ => raw,
        }
    }

    fn add(&mut self, direction: GestureDirection, length: u32) {
        if self.overflow {
            return;
        }
        if self.merge_last(direction, length) {
            return;
        }
        if self.len == RAW_STROKES {
            // 仍在绘制的最后一笔可能继续变长，只压缩已完成的笔画；此时最长一笔未定，只用固定下限。
            if !self.remove_shortest(self.len - 1, GESTURE_MIN_STROKE_PX) {
                self.overflow = true;
                return;
            }
            if self.merge_last(direction, length) {
                return;
            }
        }
        self.strokes[self.len] = Stroke { direction, length };
        self.len += 1;
    }

    fn merge_last(&mut self, direction: GestureDirection, length: u32) -> bool {
        match self
            .len
            .checked_sub(1)
            .map(|index| &mut self.strokes[index])
        {
            Some(last) if last.direction == direction => {
                last.length = last.length.saturating_add(length);
                true
            }
            _ => false,
        }
    }

    /// 删除 `..limit` 中最短且不足 `floor` 的一笔，再合并相邻同向笔画。
    /// 同长时先删两侧同向的一笔（删除后两侧并为一笔，长度不丢失），再取最早。
    fn remove_shortest(&mut self, limit: usize, floor: u32) -> bool {
        let bridges = |index: usize| {
            index > 0
                && index + 1 < self.len
                && self.strokes[index - 1].direction == self.strokes[index + 1].direction
        };
        let Some(index) = (0..limit.min(self.len))
            .filter(|&index| self.strokes[index].length < floor)
            .min_by_key(|&index| (self.strokes[index].length, !bridges(index), index))
        else {
            return false;
        };
        self.strokes.copy_within(index + 1..self.len, index);
        self.len -= 1;
        if index > 0
            && index < self.len
            && self.strokes[index - 1].direction == self.strokes[index].direction
        {
            self.strokes[index - 1].length = self.strokes[index - 1]
                .length
                .saturating_add(self.strokes[index].length);
            self.strokes.copy_within(index + 1..self.len, index);
            self.len -= 1;
        }
        true
    }

    /// 笔画下限：固定像素与最长一笔的 1/[`GESTURE_STROKE_RATIO`] 中较大者。
    fn floor(&self) -> u32 {
        let longest = self.strokes[..self.len]
            .iter()
            .map(|stroke| stroke.length)
            .max()
            .unwrap_or(0);
        GESTURE_MIN_STROKE_PX.max(longest / GESTURE_STROKE_RATIO)
    }

    fn path(&self) -> Option<GesturePath> {
        if self.overflow {
            return None;
        }
        let mut log = *self;
        // 合并会让最长一笔变长，下限随之重算；每轮至少删除一笔，循环有界。
        while log.remove_shortest(log.len, log.floor()) {}
        let mut steps = [GestureDirection::Up; MAX_GESTURE_STEPS];
        let kept = steps.get_mut(..log.len)?;
        for (step, stroke) in kept.iter_mut().zip(&log.strokes) {
            *step = stroke.direction;
        }
        GesturePath::new(kept).ok()
    }
}

/// 一次手势的两份识别读数；超过容量或没有有效笔画时为 `None`。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GestureReading {
    pub cardinal: Option<GesturePath>,
    pub octal: Option<GesturePath>,
}

impl GestureReading {
    /// 含斜向的手势比较八向读数，其余比较四向读数。
    pub fn matches(&self, path: &GesturePath) -> bool {
        if path.has_diagonal() {
            self.octal.as_ref() == Some(path)
        } else {
            self.cardinal.as_ref() == Some(path)
        }
    }

    /// 供界面显示的读数：八向读数含斜向时优先，否则取四向读数。
    pub fn preferred(&self) -> Option<GesturePath> {
        self.octal
            .filter(GesturePath::has_diagonal)
            .or(self.cardinal)
    }
}

/// 单个采样段的位移限幅：远大于任何屏幕，又保证平方与正切比较不溢出。
fn clamp_delta(value: i64) -> i64 {
    const LIMIT: i64 = 1 << 20;
    value.clamp(-LIMIT, LIMIT)
}

/// 固定容量的轨迹识别器；只接收显式提供的物理坐标。
#[derive(Clone, Debug)]
pub struct GestureTracker {
    origin: (i32, i32),
    anchor: (i32, i32),
    started: bool,
    cardinal: StrokeLog,
    octal: StrokeLog,
}

impl GestureTracker {
    pub fn new(origin: (i32, i32)) -> Self {
        Self {
            origin,
            anchor: origin,
            started: false,
            cardinal: StrokeLog::new(),
            octal: StrokeLog::new(),
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
        if !self.started {
            let (dx, dy) = delta(self.origin);
            let start = i64::from(GESTURE_START_PX);
            self.started =
                dx.abs() >= start || dy.abs() >= start || dx * dx + dy * dy >= start * start;
        }
        let (dx, dy) = delta(self.anchor);
        // 各轴先与有界段长比较，只有较小差值才平方，极端坐标不会溢出。
        if dx.abs() < SEGMENT_PX
            && dy.abs() < SEGMENT_PX
            && dx * dx + dy * dy < SEGMENT_PX * SEGMENT_PX
        {
            return;
        }
        // 方向与长度只取决于限幅后的位移；极端坐标下乘积仍在 i64 范围内。
        let (dx, dy) = (clamp_delta(dx), clamp_delta(dy));
        let length = (dx * dx + dy * dy).isqrt() as u32;
        let cardinal = self.cardinal.settle(
            GestureDirection::cardinal(dx, dy),
            dx,
            dy,
            CARDINAL_STICKY_TAN,
        );
        self.cardinal.add(cardinal, length);
        let octal = self
            .octal
            .settle(GestureDirection::octal(dx, dy), dx, dy, OCTAL_STICKY_TAN);
        self.octal.add(octal, length);
        self.anchor = position;
    }

    pub fn reading(&self) -> GestureReading {
        GestureReading {
            cardinal: self.cardinal.path(),
            octal: self.octal.path(),
        }
    }
}
