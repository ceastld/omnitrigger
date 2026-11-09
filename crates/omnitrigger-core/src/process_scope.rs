//! 前台范围：只比较调用方提供的映像文件名与窗口角色，不查询系统或文件。
use std::collections::BTreeSet;
use std::fmt;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProcessScope {
    /// 缺省匹配真实映像名；窗口上下文域显式区分桌面、任务栏和应用窗口。
    pub identity: ProcessIdentity,
    /// 空列表表示允许所有已知进程；整个范围为空时也允许未知上下文。
    pub allow: Vec<String>,
    pub deny: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProcessIdentity {
    #[default]
    Executable,
    ShellContext,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WindowRole {
    #[default]
    Unknown,
    Application,
    Desktop,
    Taskbar,
}

impl WindowRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Application => "application",
            Self::Desktop => "desktop",
            Self::Taskbar => "taskbar",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProcessScopeError {
    TooManyNames,
    InvalidName { name: String },
    DuplicateName { name: String },
    EmptyRange,
}

impl fmt::Display for ProcessScopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyNames => f.write_str("进程白名单和黑名单合计最多 64 项。"),
            Self::InvalidName { name } => write!(
                f,
                "进程名无效：{name:?}；请使用文件名，可省略 .exe，不支持路径或通配符。"
            ),
            Self::DuplicateName { name } => write!(f, "同一进程列表包含重复名称：{name:?}。"),
            Self::EmptyRange => f.write_str("进程白名单中的全部应用均被黑名单排除。"),
        }
    }
}
impl std::error::Error for ProcessScopeError {}

/// 两个范围不能合并为单个范围的原因。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScopeIntersectionError {
    /// 两侧都有限制但匹配域不同，单个范围无法同时表达。
    IdentityMismatch,
    /// 两侧白名单都非空且没有共同名称，合并后永远不会命中。
    DisjointAllow,
}

impl fmt::Display for ScopeIntersectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::IdentityMismatch => "两者的匹配域（进程名／窗口上下文）不同",
            Self::DisjointAllow => "两者的应用白名单没有共同名称",
        })
    }
}
impl std::error::Error for ScopeIntersectionError {}

/// 进程名的规范形式：小写并去掉可选的 `.exe`；名称无效时返回 `None`。
/// 与范围校验共用同一规则，供配置层判断重复名称。
pub fn canonical_process_name(name: &str) -> Option<String> {
    normalize_name(name)
}

pub(crate) fn normalize_name(name: &str) -> Option<String> {
    if name.is_empty()
        || name.len() > 260
        || name.trim() != name
        || name
            .chars()
            .any(|c| c.is_control() || "/\\:*?;\"<>|".contains(c))
    {
        return None;
    }
    let mut name = name.to_lowercase();
    if name.ends_with(".exe") {
        name.truncate(name.len() - 4);
    }
    (!name.is_empty() && name != "." && name != "..").then_some(name)
}

impl ProcessScope {
    pub fn is_global(&self) -> bool {
        self.allow.is_empty() && self.deny.is_empty()
    }

    /// 按注册时的规则校验名单，不修改自身；用于尚未被绑定使用的定义。
    pub fn validate(&self) -> Result<(), ProcessScopeError> {
        self.clone().normalize()
    }

    /// 同时满足两个范围的单个范围：白名单取交集（一侧为空时沿用另一侧），黑名单取并集。
    /// 全局范围不限制，合并时直接返回另一侧。名称按规范形式比较并保留左侧写法；
    /// 无效名称留给 [`Self::validate`] 报错。
    pub fn intersect(&self, other: &Self) -> Result<Self, ScopeIntersectionError> {
        if other.is_global() {
            return Ok(self.clone());
        }
        if self.is_global() {
            return Ok(other.clone());
        }
        if self.identity != other.identity {
            return Err(ScopeIntersectionError::IdentityMismatch);
        }
        let key = |name: &String| normalize_name(name).unwrap_or_else(|| name.clone());
        let allow = if other.allow.is_empty() {
            self.allow.clone()
        } else if self.allow.is_empty() {
            other.allow.clone()
        } else {
            let theirs: BTreeSet<String> = other.allow.iter().map(key).collect();
            let both: Vec<String> = self
                .allow
                .iter()
                .filter(|name| theirs.contains(&key(name)))
                .cloned()
                .collect();
            if both.is_empty() {
                return Err(ScopeIntersectionError::DisjointAllow);
            }
            both
        };
        let mut deny = self.deny.clone();
        let mut seen: BTreeSet<String> = deny.iter().map(key).collect();
        for name in &other.deny {
            if seen.insert(key(name)) {
                deny.push(name.clone());
            }
        }
        Ok(Self {
            identity: self.identity,
            allow,
            deny,
        })
    }

    /// 规范化后的副本：名称小写并去掉 `.exe`，按注册规则校验。
    pub fn normalized(&self) -> Result<Self, ProcessScopeError> {
        let mut scope = self.clone();
        scope.normalize()?;
        Ok(scope)
    }

    /// 判断给定上下文是否命中，规则与注册后的输入匹配相同。
    /// 范围应先经 [`Self::normalized`]；进程名在此规范化，无效进程名视为未知。
    pub fn matches(&self, process: Option<&str>, role: WindowRole) -> bool {
        let process = process.and_then(normalize_name);
        self.matches_context(process.as_deref(), role)
    }

    /// 两个已规范化的范围是否存在同时命中的上下文；与注册冲突检查相同。
    pub fn intersects(&self, other: &Self) -> bool {
        self.overlaps(other)
    }

    /// 两个已规范化的范围中，命中 `self` 的每个上下文是否也命中 `other`。
    /// 按标准 Windows 外壳判断：桌面与任务栏窗口只属于 `explorer` 进程。
    /// 仅供编辑界面推断沿用关系；运行时匹配和冲突检查不依赖这一假设。
    pub fn implies(&self, other: &Self) -> bool {
        self.sample_contexts(other)
            .filter(|(name, role)| *role == WindowRole::Application || name == "explorer")
            .all(|(name, role)| {
                !self.matches_context(Some(&name), role) || other.matches_context(Some(&name), role)
            })
    }

    /// 范围优先级：非空白名单高于全局或只含黑名单的范围。
    pub fn specificity(&self) -> u8 {
        self.priority()
    }

    pub(crate) fn priority(&self) -> u8 {
        u8::from(!self.allow.is_empty())
    }

    pub(crate) fn needs_role(&self) -> bool {
        self.identity == ProcessIdentity::ShellContext && !self.is_global()
    }

    pub(crate) fn normalize(&mut self) -> Result<(), ProcessScopeError> {
        if self.allow.len() + self.deny.len() > 64 {
            return Err(ProcessScopeError::TooManyNames);
        }
        for names in [&mut self.allow, &mut self.deny] {
            let mut seen = BTreeSet::new();
            for name in names.iter_mut() {
                let canonical = normalize_name(name)
                    .ok_or_else(|| ProcessScopeError::InvalidName { name: name.clone() })?;
                if !seen.insert(canonical.clone()) {
                    return Err(ProcessScopeError::DuplicateName { name: name.clone() });
                }
                *name = canonical;
            }
        }
        if !self.allow.is_empty() && self.allow.iter().all(|name| self.deny.contains(name)) {
            return Err(ProcessScopeError::EmptyRange);
        }
        Ok(())
    }

    pub(crate) fn matches_context(&self, process: Option<&str>, role: WindowRole) -> bool {
        if self.is_global() {
            return true;
        }
        let Some(process) = process else {
            return false;
        };
        let process = match self.identity {
            ProcessIdentity::Executable => process,
            ProcessIdentity::ShellContext => match role {
                WindowRole::Unknown => return false,
                WindowRole::Application => process,
                WindowRole::Desktop => "desktop",
                WindowRole::Taskbar => "taskbar",
            },
        };
        (self.allow.is_empty() || self.allow.iter().any(|name| name == process))
            && !self.deny.iter().any(|name| name == process)
    }

    pub(crate) fn overlaps(&self, other: &Self) -> bool {
        self.sample_contexts(other).any(|(name, role)| {
            self.matches_context(Some(&name), role) && other.matches_context(Some(&name), role)
        })
    }

    /// 已知上下文的代表样本：匹配结果只会在名单名称、两个角色名称及名单外进程之间变化，
    /// 有限枚举即可覆盖。未知上下文只命中全局范围，无需列出。
    fn sample_contexts(&self, other: &Self) -> impl Iterator<Item = (String, WindowRole)> {
        let mut names: BTreeSet<String> = self
            .allow
            .iter()
            .chain(&self.deny)
            .chain(&other.allow)
            .chain(&other.deny)
            .cloned()
            .collect();
        names.extend(["desktop", "taskbar", "explorer"].map(String::from));
        let mut outside = "__other_process__".to_string();
        while names.contains(&outside) {
            outside.push('_');
        }
        names.insert(outside);
        names.into_iter().flat_map(|name| {
            [
                WindowRole::Application,
                WindowRole::Desktop,
                WindowRole::Taskbar,
            ]
            .map(|role| (name.clone(), role))
        })
    }
}
