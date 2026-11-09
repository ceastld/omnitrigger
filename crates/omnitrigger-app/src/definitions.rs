//! 命名定义的通用内核：名称规则、引用归属、解析顺序与循环检测。
//!
//! 应用集合（值常量）与场景（规则常量）等定义种类共用这些规则；
//! 各种类只需说明“一个定义引用了哪些同类定义”以及“如何把引用合成为结果”。
//! 定义只在配置边界展开，核心看不到名称与引用。
use std::collections::BTreeMap;
use std::fmt;

pub const MAX_DEFINITIONS: usize = 64;
const MAX_NAME_LEN: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DefinitionKind {
    AppSet,
    Scene,
}

impl DefinitionKind {
    fn label(self) -> &'static str {
        match self {
            Self::AppSet => "应用集合",
            Self::Scene => "场景",
        }
    }

    /// 配置原文中引用该定义的写法：集合在名单里以 `@` 区分进程名，场景字段直接写名称。
    pub fn reference(self, name: &str) -> String {
        match self {
            Self::AppSet => format!("@{name}"),
            Self::Scene => name.into(),
        }
    }
}

/// 发生问题的配置位置。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Owner {
    Binding(u64),
    Definition(DefinitionKind, String),
}

impl fmt::Display for Owner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binding(id) => write!(f, "触发器 #{id}"),
            Self::Definition(kind, name) => {
                write!(f, "{} {}", kind.label(), kind.reference(name))
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DefinitionError {
    TooMany {
        kind: DefinitionKind,
    },
    InvalidName {
        kind: DefinitionKind,
        name: String,
    },
    Unknown {
        kind: DefinitionKind,
        owner: Owner,
        name: String,
    },
    Cycle {
        kind: DefinitionKind,
        path: Vec<String>,
    },
}

impl fmt::Display for DefinitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooMany { kind } => {
                write!(f, "{}最多 {MAX_DEFINITIONS} 个。", kind.label())
            }
            Self::InvalidName { kind, name } => write!(
                f,
                "{}名无效：{name:?}；只能使用小写字母、数字、- 和 _，最多 {MAX_NAME_LEN} 个字符。",
                kind.label()
            ),
            Self::Unknown { kind, owner, name } => write!(
                f,
                "{owner} 引用了未定义的{} {}。",
                kind.label(),
                kind.reference(name)
            ),
            Self::Cycle { kind, path } => write!(
                f,
                "{}存在循环引用：{}。",
                kind.label(),
                path.iter()
                    .map(|name| kind.reference(name))
                    .collect::<Vec<_>>()
                    .join(" → ")
            ),
        }
    }
}

impl std::error::Error for DefinitionError {}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_LEN
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// 查找被引用的定义；未定义时返回指明引用位置的错误。
pub fn lookup<'a, T>(
    kind: DefinitionKind,
    definitions: &'a BTreeMap<String, T>,
    owner: &Owner,
    name: &str,
) -> Result<&'a T, DefinitionError> {
    definitions
        .get(name)
        .ok_or_else(|| DefinitionError::Unknown {
            kind,
            owner: owner.clone(),
            name: name.into(),
        })
}

/// 校验名称、引用与循环，返回被引用者在前的解析顺序。
/// `references` 给出一个定义直接引用的同类定义名称。
pub fn resolution_order<'a, T>(
    kind: DefinitionKind,
    definitions: &'a BTreeMap<String, T>,
    references: impl Fn(&'a T) -> Vec<&'a str>,
) -> Result<Vec<&'a str>, DefinitionError> {
    if definitions.len() > MAX_DEFINITIONS {
        return Err(DefinitionError::TooMany { kind });
    }
    let mut edges = BTreeMap::new();
    for (name, definition) in definitions {
        if !valid_name(name) {
            return Err(DefinitionError::InvalidName {
                kind,
                name: name.clone(),
            });
        }
        let targets = references(definition);
        let owner = Owner::Definition(kind, name.clone());
        for target in &targets {
            lookup(kind, definitions, &owner, target)?;
        }
        edges.insert(name.as_str(), targets);
    }
    let mut state = BTreeMap::new();
    let mut order = Vec::with_capacity(definitions.len());
    for name in definitions.keys() {
        visit(kind, name, &edges, &mut state, &mut Vec::new(), &mut order)?;
    }
    Ok(order)
}

fn visit<'a>(
    kind: DefinitionKind,
    name: &'a str,
    edges: &BTreeMap<&'a str, Vec<&'a str>>,
    done: &mut BTreeMap<&'a str, bool>,
    path: &mut Vec<&'a str>,
    order: &mut Vec<&'a str>,
) -> Result<(), DefinitionError> {
    match done.get(name) {
        Some(true) => return Ok(()),
        Some(false) => {
            let start = path.iter().position(|item| *item == name).unwrap_or(0);
            let mut cycle: Vec<String> = path[start..].iter().map(|item| (*item).into()).collect();
            cycle.push(name.into());
            return Err(DefinitionError::Cycle { kind, path: cycle });
        }
        None => {}
    }
    done.insert(name, false);
    path.push(name);
    for &target in &edges[name] {
        visit(kind, target, edges, done, path, order)?;
    }
    path.pop();
    done.insert(name, true);
    order.push(name);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(edges: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
        edges
            .iter()
            .map(|(name, targets)| {
                (
                    (*name).to_string(),
                    targets.iter().map(|target| (*target).to_string()).collect(),
                )
            })
            .collect()
    }

    fn order(definitions: &BTreeMap<String, Vec<String>>) -> Result<Vec<&str>, DefinitionError> {
        resolution_order(DefinitionKind::Scene, definitions, |targets| {
            targets.iter().map(String::as_str).collect()
        })
    }

    #[test]
    fn referenced_definitions_come_first() {
        let definitions = graph(&[
            ("work", &["browser", "base"]),
            ("browser", &["base"]),
            ("base", &[]),
        ]);
        assert_eq!(order(&definitions).unwrap(), ["base", "browser", "work"]);
    }

    #[test]
    fn names_references_and_cycles_are_reported_with_their_location() {
        assert_eq!(
            order(&graph(&[("Work", &[])])),
            Err(DefinitionError::InvalidName {
                kind: DefinitionKind::Scene,
                name: "Work".into()
            })
        );
        assert_eq!(
            order(&graph(&[("work", &["missing"])]))
                .unwrap_err()
                .to_string(),
            "场景 work 引用了未定义的场景 missing。"
        );
        assert_eq!(
            order(&graph(&[("a", &["b"]), ("b", &["c"]), ("c", &["a"])]))
                .unwrap_err()
                .to_string(),
            "场景存在循环引用：a → b → c → a。"
        );
        let sets = graph(&[("a", &["a"])]);
        assert_eq!(
            resolution_order(DefinitionKind::AppSet, &sets, |targets| {
                targets.iter().map(String::as_str).collect()
            })
            .unwrap_err()
            .to_string(),
            "应用集合存在循环引用：@a → @a。"
        );
        let many: BTreeMap<String, Vec<String>> = (0..=MAX_DEFINITIONS)
            .map(|index| (format!("s{index}"), vec![]))
            .collect();
        assert_eq!(
            order(&many),
            Err(DefinitionError::TooMany {
                kind: DefinitionKind::Scene
            })
        );
    }
}
