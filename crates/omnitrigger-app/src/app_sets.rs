//! 应用集合（值常量）：在配置中定义一次进程名列表，在白黑名单中以 `@名称` 引用。
//!
//! 集合只保存名称，不带匹配域；放在白名单还是黑名单、按进程名还是窗口上下文匹配，
//! 由引用它的范围决定。名称、引用与循环规则由 [`crate::definitions`] 统一处理。
//! 只在配置边界展开，核心仍只看到展开后的进程名；保存时写回原文中的引用。
use crate::definitions::{self, DefinitionError, DefinitionKind, Owner};
use omnitrigger_core::{canonical_process_name, ProcessIdentity, ProcessScope};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

pub const MAX_SET_ENTRIES: usize = 64;
const REFERENCE_PREFIX: char = '@';

/// 集合名 → 条目；条目为进程名或 `@其他集合`。
pub type AppSets = BTreeMap<String, Vec<String>>;

/// 可以引用集合的范围字段；错误信息按配置中的字段路径指明位置。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScopeField {
    Process,
    Pointer,
    Tap,
    Capture,
}

impl ScopeField {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Process => "process_scope",
            Self::Pointer => "pointer_scope",
            Self::Tap => "keyboard.tap_scope",
            Self::Capture => "keyboard.retained_prefix.capture_scope",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppSetError {
    Definition(DefinitionError),
    TooManyEntries {
        set: String,
    },
    InvalidEntry {
        set: String,
        entry: String,
    },
    DuplicateEntry {
        set: String,
        entry: String,
    },
    /// 范围字段引用了未定义的集合。
    UnknownReference {
        owner: Owner,
        field: ScopeField,
        name: String,
    },
    /// 白名单只引用了空集合；展开后为空会变成“所有应用”，因此拒绝。
    EmptyAllow {
        owner: Owner,
        field: ScopeField,
    },
    /// 旧版配置中以 `@` 开头的进程名在版本 24 起表示引用，不能原样升级保存。
    LiteralReference {
        binding: u64,
        field: ScopeField,
        name: String,
    },
}

impl From<DefinitionError> for AppSetError {
    fn from(error: DefinitionError) -> Self {
        Self::Definition(error)
    }
}

impl fmt::Display for AppSetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Definition(error) => error.fmt(f),
            Self::TooManyEntries { set } => {
                write!(f, "应用集合 @{set} 最多 {MAX_SET_ENTRIES} 项。")
            }
            Self::InvalidEntry { set, entry } => write!(
                f,
                "应用集合 @{set} 中的进程名无效：{entry:?}；请使用文件名，可省略 .exe，不支持路径或通配符。"
            ),
            Self::DuplicateEntry { set, entry } => {
                write!(f, "应用集合 @{set} 包含重复条目：{entry:?}。")
            }
            Self::UnknownReference { owner, field, name } => write!(
                f,
                "{owner} 的 {} 引用了未定义的应用集合 @{name}。",
                field.as_str()
            ),
            Self::EmptyAllow { owner, field } => write!(
                f,
                "{owner} 的 {} 白名单引用的集合展开后为空；空白名单表示所有应用，请在集合中添加名称或移除该引用。",
                field.as_str()
            ),
            Self::LiteralReference {
                binding,
                field,
                name,
            } => write!(
                f,
                "触发器 #{binding} 的 {} 包含以 @ 开头的进程名 {name:?}；版本 24 起 @ 表示集合引用，请先在配置中改名。",
                field.as_str()
            ),
        }
    }
}

impl std::error::Error for AppSetError {}

fn reference(entry: &str) -> Option<&str> {
    entry.strip_prefix(REFERENCE_PREFIX)
}

/// 校验集合定义：名称、引用、循环，以及每个集合自身的条目。
pub fn validate(sets: &AppSets) -> Result<(), AppSetError> {
    definitions::resolution_order(DefinitionKind::AppSet, sets, |entries| {
        entries
            .iter()
            .filter_map(|entry| reference(entry))
            .collect()
    })?;
    for (set, entries) in sets {
        if entries.len() > MAX_SET_ENTRIES {
            return Err(AppSetError::TooManyEntries { set: set.clone() });
        }
        let mut seen = std::collections::BTreeSet::new();
        for entry in entries {
            let key = match reference(entry) {
                Some(_) => entry.clone(),
                None => canonical_process_name(entry).ok_or_else(|| AppSetError::InvalidEntry {
                    set: set.clone(),
                    entry: entry.clone(),
                })?,
            };
            if !seen.insert(key) {
                return Err(AppSetError::DuplicateEntry {
                    set: set.clone(),
                    entry: entry.clone(),
                });
            }
        }
    }
    Ok(())
}

/// 按书写顺序取出集合内的进程名；集合已通过 [`validate`]，不含循环。
fn flatten(set: &str, sets: &AppSets, out: &mut Vec<String>) {
    for entry in &sets[set] {
        match reference(entry) {
            Some(name) => flatten(name, sets, out),
            None => out.push(entry.clone()),
        }
    }
}

/// 展开一侧名单。引用带来的重复按集合并集合并；直接写出的重复名称原样保留，
/// 仍由核心范围校验报错，与未使用集合时的行为一致。
fn expand_side(
    entries: &[String],
    sets: &AppSets,
    owner: &Owner,
    field: ScopeField,
) -> Result<Vec<String>, AppSetError> {
    let mut out = Vec::new();
    // 规范名 → 是否来自集合引用。
    let mut origin: BTreeMap<String, bool> = BTreeMap::new();
    for entry in entries {
        if let Some(name) = reference(entry) {
            if !sets.contains_key(name) {
                return Err(AppSetError::UnknownReference {
                    owner: owner.clone(),
                    field,
                    name: name.into(),
                });
            }
            let mut names = Vec::new();
            flatten(name, sets, &mut names);
            for name in names {
                let key = canonical_process_name(&name).unwrap_or_else(|| name.clone());
                if let std::collections::btree_map::Entry::Vacant(slot) = origin.entry(key) {
                    slot.insert(true);
                    out.push(name);
                }
            }
        } else if let Some(key) = canonical_process_name(entry) {
            if origin.get(&key) != Some(&true) {
                origin.insert(key, false);
                out.push(entry.clone());
            }
        } else {
            out.push(entry.clone());
        }
    }
    Ok(out)
}

/// 配置原文中含集合引用的白黑名单，以及读取时的展开结果。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScopeSource {
    allow: Vec<String>,
    deny: Vec<String>,
    expanded_allow: Vec<String>,
    expanded_deny: Vec<String>,
}

impl ScopeSource {
    /// 范围仍等于读取时的展开结果时返回原文条目；范围已被改写则返回 `None`，按实际名称保存。
    pub fn authored_for(&self, scope: &ProcessScope) -> Option<(&[String], &[String])> {
        (scope.allow == self.expanded_allow && scope.deny == self.expanded_deny)
            .then_some((&self.allow, &self.deny))
    }
}

/// 一条绑定中各范围字段的集合引用原文；字段未使用引用时为 `None`。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ScopeSources {
    pub process: Option<ScopeSource>,
    pub pointer: Option<ScopeSource>,
    pub tap: Option<ScopeSource>,
    pub capture: Option<ScopeSource>,
}

impl ScopeSources {
    pub fn get(&self, field: ScopeField) -> Option<&ScopeSource> {
        match field {
            ScopeField::Process => self.process.as_ref(),
            ScopeField::Pointer => self.pointer.as_ref(),
            ScopeField::Tap => self.tap.as_ref(),
            ScopeField::Capture => self.capture.as_ref(),
        }
    }

    pub fn set(&mut self, field: ScopeField, source: Option<ScopeSource>) {
        match field {
            ScopeField::Process => self.process = source,
            ScopeField::Pointer => self.pointer = source,
            ScopeField::Tap => self.tap = source,
            ScopeField::Capture => self.capture = source,
        }
    }
}

/// 配置中的范围原文；名单条目可以是进程名或 `@集合`。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessScopeDto {
    #[serde(default, skip_serializing_if = "ProcessIdentityDto::is_default")]
    pub identity: ProcessIdentityDto,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny: Vec<String>,
}

impl ProcessScopeDto {
    pub fn is_global(&self) -> bool {
        self.allow.is_empty() && self.deny.is_empty() && self.identity.is_default()
    }

    /// 范围未被改写时写回原文中的集合引用，否则写实际名称。
    pub fn encode(scope: &ProcessScope, source: Option<&ScopeSource>) -> Self {
        let (allow, deny) = display_names(scope, source);
        Self {
            identity: scope.identity.into(),
            allow: allow.to_vec(),
            deny: deny.to_vec(),
        }
    }

    /// 不展开引用，名称一律按字面进程名处理；用于版本 24 之前的配置。
    pub fn literal(self) -> ProcessScope {
        ProcessScope {
            identity: self.identity.into(),
            allow: self.allow,
            deny: self.deny,
        }
    }

    /// 展开 `@集合` 引用；不含引用时原样返回且不附带原文。
    pub fn resolve(
        self,
        sets: &AppSets,
        owner: &Owner,
        field: ScopeField,
    ) -> Result<(ProcessScope, Option<ScopeSource>), AppSetError> {
        let identity = self.identity.into();
        if !self
            .allow
            .iter()
            .chain(&self.deny)
            .any(|entry| reference(entry).is_some())
        {
            return Ok((self.literal(), None));
        }
        let expanded_allow = expand_side(&self.allow, sets, owner, field)?;
        let expanded_deny = expand_side(&self.deny, sets, owner, field)?;
        if !self.allow.is_empty() && expanded_allow.is_empty() {
            return Err(AppSetError::EmptyAllow {
                owner: owner.clone(),
                field,
            });
        }
        Ok((
            ProcessScope {
                identity,
                allow: expanded_allow.clone(),
                deny: expanded_deny.clone(),
            },
            Some(ScopeSource {
                allow: self.allow,
                deny: self.deny,
                expanded_allow,
                expanded_deny,
            }),
        ))
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessIdentityDto {
    #[default]
    Executable,
    ShellContext,
}

impl ProcessIdentityDto {
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Executable)
    }
}

impl From<ProcessIdentity> for ProcessIdentityDto {
    fn from(value: ProcessIdentity) -> Self {
        match value {
            ProcessIdentity::Executable => Self::Executable,
            ProcessIdentity::ShellContext => Self::ShellContext,
        }
    }
}

impl From<ProcessIdentityDto> for ProcessIdentity {
    fn from(value: ProcessIdentityDto) -> Self {
        match value {
            ProcessIdentityDto::Executable => Self::Executable,
            ProcessIdentityDto::ShellContext => Self::ShellContext,
        }
    }
}

/// 按实际名称写出的范围不能包含以 `@` 开头的名称，否则重新读取时会被当作引用。
pub fn check_saveable(
    binding: u64,
    field: ScopeField,
    scope: &ProcessScope,
    source: Option<&ScopeSource>,
) -> Result<(), AppSetError> {
    if source.is_some_and(|source| source.authored_for(scope).is_some()) {
        return Ok(());
    }
    match scope
        .allow
        .iter()
        .chain(&scope.deny)
        .find(|name| reference(name).is_some())
    {
        Some(name) => Err(AppSetError::LiteralReference {
            binding,
            field,
            name: name.clone(),
        }),
        None => Ok(()),
    }
}

/// 列表与详情中显示的名单：沿用原文中的 `@集合` 写法。
pub fn display_names<'a>(
    scope: &'a ProcessScope,
    source: Option<&'a ScopeSource>,
) -> (&'a [String], &'a [String]) {
    source
        .and_then(|source| source.authored_for(scope))
        .unwrap_or((&scope.allow, &scope.deny))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sets(json: serde_json::Value) -> AppSets {
        serde_json::from_value(json).unwrap()
    }

    fn strings(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).into()).collect()
    }

    fn dto(allow: &[&str], deny: &[&str]) -> ProcessScopeDto {
        ProcessScopeDto {
            identity: Default::default(),
            allow: strings(allow),
            deny: strings(deny),
        }
    }

    const OWNER: Owner = Owner::Binding(7);

    #[test]
    fn nested_sets_expand_in_order_and_merge_duplicates_from_references() {
        let sets = sets(serde_json::json!({
            "chromium": ["chrome", "msedge"],
            "browsers": ["@chromium", "firefox"]
        }));
        validate(&sets).unwrap();
        let (scope, source) = dto(&["Chrome.exe", "@browsers", "code"], &["@chromium"])
            .resolve(&sets, &OWNER, ScopeField::Process)
            .unwrap();
        assert_eq!(
            scope.allow,
            strings(&["Chrome.exe", "msedge", "firefox", "code"])
        );
        assert_eq!(scope.deny, strings(&["chrome", "msedge"]));
        let source = source.unwrap();
        assert_eq!(
            source.authored_for(&scope),
            Some((
                &strings(&["Chrome.exe", "@browsers", "code"])[..],
                &strings(&["@chromium"])[..]
            ))
        );
        let changed = ProcessScope {
            allow: strings(&["code"]),
            ..scope
        };
        assert_eq!(source.authored_for(&changed), None);
    }

    #[test]
    fn literal_duplicates_are_left_for_scope_validation() {
        let (scope, _) = dto(&["code", "Code.exe", "@editors"], &[])
            .resolve(
                &sets(serde_json::json!({"editors": ["code", "notepad"]})),
                &OWNER,
                ScopeField::Pointer,
            )
            .unwrap();
        assert_eq!(scope.allow, strings(&["code", "Code.exe", "notepad"]));
    }

    #[test]
    fn scopes_without_references_keep_no_source() {
        let (scope, source) = dto(&["notepad"], &["game"])
            .resolve(&AppSets::new(), &OWNER, ScopeField::Tap)
            .unwrap();
        assert!(source.is_none());
        assert_eq!(scope.allow, strings(&["notepad"]));
    }

    #[test]
    fn empty_allow_after_expansion_is_rejected_but_empty_deny_is_not() {
        let sets = sets(serde_json::json!({"none": []}));
        validate(&sets).unwrap();
        let owner = Owner::Definition(DefinitionKind::Scene, "quiet".into());
        assert_eq!(
            dto(&["@none"], &[])
                .resolve(&sets, &owner, ScopeField::Capture)
                .unwrap_err()
                .to_string(),
            "场景 quiet 的 keyboard.retained_prefix.capture_scope 白名单引用的集合展开后为空；空白名单表示所有应用，请在集合中添加名称或移除该引用。"
        );
        let (scope, _) = dto(&[], &["@none"])
            .resolve(&sets, &OWNER, ScopeField::Process)
            .unwrap();
        assert!(scope.is_global());
    }

    #[test]
    fn unknown_reference_names_the_owner() {
        let error = dto(&["@browsers"], &[])
            .resolve(&AppSets::new(), &Owner::Binding(12), ScopeField::Pointer)
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "触发器 #12 的 pointer_scope 引用了未定义的应用集合 @browsers。"
        );
    }

    #[test]
    fn set_definitions_are_validated() {
        for (json, expected) in [
            (
                serde_json::json!({"a": ["bad/path"]}),
                AppSetError::InvalidEntry {
                    set: "a".into(),
                    entry: "bad/path".into(),
                },
            ),
            (
                serde_json::json!({"a": ["chrome", "Chrome.exe"]}),
                AppSetError::DuplicateEntry {
                    set: "a".into(),
                    entry: "Chrome.exe".into(),
                },
            ),
            (
                serde_json::json!({"a": ["@missing"]}),
                AppSetError::Definition(DefinitionError::Unknown {
                    kind: DefinitionKind::AppSet,
                    owner: Owner::Definition(DefinitionKind::AppSet, "a".into()),
                    name: "missing".into(),
                }),
            ),
            (
                serde_json::json!({"a": ["@b"], "b": ["@a"]}),
                AppSetError::Definition(DefinitionError::Cycle {
                    kind: DefinitionKind::AppSet,
                    path: strings(&["a", "b", "a"]),
                }),
            ),
        ] {
            assert_eq!(validate(&sets(json)), Err(expected));
        }
        let long: AppSets = [(
            "a".to_string(),
            (0..=MAX_SET_ENTRIES).map(|i| format!("p{i}")).collect(),
        )]
        .into();
        assert_eq!(
            validate(&long),
            Err(AppSetError::TooManyEntries { set: "a".into() })
        );
    }

    #[test]
    fn explicit_names_starting_with_at_cannot_be_saved() {
        let scope = ProcessScope {
            allow: strings(&["@legacy"]),
            ..Default::default()
        };
        assert!(matches!(
            check_saveable(4, ScopeField::Process, &scope, None),
            Err(AppSetError::LiteralReference { binding: 4, .. })
        ));
        let sets = sets(serde_json::json!({"legacy": ["notepad"]}));
        let (scope, source) = dto(&["@legacy"], &[])
            .resolve(&sets, &OWNER, ScopeField::Process)
            .unwrap();
        assert!(check_saveable(4, ScopeField::Process, &scope, source.as_ref()).is_ok());
    }
}
