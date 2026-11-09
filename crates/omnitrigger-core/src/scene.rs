//! 场景：输入发生时上下文须同时满足的一组约束。
//!
//! 每个上下文维度独立约束，未约束的维度不限制；合并场景即各维度分别取交集，
//! 交集无法用单个约束表达或永远不会命中时返回指明维度的结构化冲突。
//! 场景只是 `TriggerSpec` 中上下文条件的成组视图，注册、冲突检查和匹配仍按各字段进行。
//! 新增维度（例如浏览器站点）时在此增加字段，并同步补齐 `of`、`apply_to` 与 `intersect`。
use crate::{ProcessScope, ScopeIntersectionError, TriggerSpec};
use std::fmt;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Scene {
    /// 前台窗口的进程范围，对应 `TriggerSpec::process_scope`。
    pub foreground: ProcessScope,
    /// 指针所在窗口的进程范围，对应 `TriggerSpec::pointer_scope`。
    pub pointer: ProcessScope,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SceneDimension {
    Foreground,
    Pointer,
}

impl SceneDimension {
    pub fn label(self) -> &'static str {
        match self {
            Self::Foreground => "前台应用",
            Self::Pointer => "指针窗口",
        }
    }
}

/// 合并两个场景时某一维度无法取交集。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SceneConflict {
    pub dimension: SceneDimension,
    pub reason: ScopeIntersectionError,
}

impl fmt::Display for SceneConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}条件无法合并：{}", self.dimension.label(), self.reason)
    }
}
impl std::error::Error for SceneConflict {}

impl Scene {
    /// 取出触发描述中的上下文条件。
    pub fn of(spec: &TriggerSpec) -> Self {
        Self {
            foreground: spec.process_scope.clone(),
            pointer: spec.pointer_scope.clone(),
        }
    }

    /// 用场景替换触发描述中的上下文条件，其他字段不变。
    pub fn apply_to(self, spec: &mut TriggerSpec) {
        spec.process_scope = self.foreground;
        spec.pointer_scope = self.pointer;
    }

    /// 所有维度都不限制时为真。
    pub fn is_unconstrained(&self) -> bool {
        self.foreground.is_global() && self.pointer.is_global()
    }

    /// 同时满足两个场景的场景；各维度分别取交集。
    pub fn intersect(&self, other: &Self) -> Result<Self, SceneConflict> {
        let conflict = |dimension| move |reason| SceneConflict { dimension, reason };
        Ok(Self {
            foreground: self
                .foreground
                .intersect(&other.foreground)
                .map_err(conflict(SceneDimension::Foreground))?,
            pointer: self
                .pointer
                .intersect(&other.pointer)
                .map_err(conflict(SceneDimension::Pointer))?,
        })
    }
}
