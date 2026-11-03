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
        // 匹配结果只会在名单名称、两个角色名称及名单外进程之间变化，有限枚举即可覆盖。
        let mut names: BTreeSet<&str> = self
            .allow
            .iter()
            .chain(&self.deny)
            .chain(&other.allow)
            .chain(&other.deny)
            .map(String::as_str)
            .collect();
        names.extend(["desktop", "taskbar", "explorer"]);
        let mut outside = "__other_process__".to_string();
        while names.contains(outside.as_str()) {
            outside.push('_');
        }
        names.insert(&outside);
        names.into_iter().any(|name| {
            [
                WindowRole::Application,
                WindowRole::Desktop,
                WindowRole::Taskbar,
            ]
            .into_iter()
            .any(|role| {
                self.matches_context(Some(name), role) && other.matches_context(Some(name), role)
            })
        })
    }
}
