//! 场景（规则常量）：把触发器的上下文条件组合成可复用的命名规则。
//!
//! 场景使用与绑定相同的上下文字段（目前为 `process_scope` 与 `pointer_scope`），名单中可引用
//! 应用集合；`extends` 组合其他场景。绑定以 `"scene"` 引用一个场景，自身的上下文字段作为附加条件。
//! 所有组合都是各维度取交集（核心 [`Scene::intersect`]）；无法表达或永远不会命中时报错，
//! 不悄悄放宽或收窄。名称、引用与循环规则由 [`crate::definitions`] 统一处理。
use crate::app_sets::{AppSetError, AppSets, ProcessScopeDto, ScopeField, ScopeSources};
use crate::definitions::{self, DefinitionError, DefinitionKind, Owner};
use omnitrigger_core::{ProcessScope, ProcessScopeError, Scene, SceneConflict, TriggerSpec};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// 配置中的场景原文。新增上下文维度时在此增加同名字段。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneDto {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extends: Vec<String>,
    #[serde(default, skip_serializing_if = "ProcessScopeDto::is_global")]
    pub process_scope: ProcessScopeDto,
    #[serde(default, skip_serializing_if = "ProcessScopeDto::is_global")]
    pub pointer_scope: ProcessScopeDto,
}

pub type SceneDefinitions = BTreeMap<String, SceneDto>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SceneError {
    Definition(DefinitionError),
    AppSet(AppSetError),
    InvalidScope {
        owner: Owner,
        field: ScopeField,
        error: ProcessScopeError,
    },
    /// `owner`（场景或绑定）与它引用的场景在某一维度无法取交集。
    Conflict {
        owner: Owner,
        scene: String,
        conflict: SceneConflict,
    },
}

impl From<DefinitionError> for SceneError {
    fn from(error: DefinitionError) -> Self {
        Self::Definition(error)
    }
}

impl From<AppSetError> for SceneError {
    fn from(error: AppSetError) -> Self {
        Self::AppSet(error)
    }
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Definition(error) => error.fmt(f),
            Self::AppSet(error) => error.fmt(f),
            Self::InvalidScope {
                owner,
                field,
                error,
            } => write!(f, "{owner} 的 {}：{error}", field.as_str()),
            Self::Conflict {
                owner,
                scene,
                conflict,
            } => write!(f, "{owner} 与场景 {scene} 的{conflict}。"),
        }
    }
}

impl std::error::Error for SceneError {}

/// 全部场景展开并合并后的结果。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CompiledScenes(BTreeMap<String, Scene>);

impl CompiledScenes {
    pub fn get(&self, name: &str) -> Option<&Scene> {
        self.0.get(name)
    }

    /// 按名称排序的全部场景。
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Scene)> {
        self.0.iter().map(|(name, scene)| (name.as_str(), scene))
    }
}

fn scope(
    dto: &ProcessScopeDto,
    sets: &AppSets,
    owner: &Owner,
    field: ScopeField,
) -> Result<ProcessScope, SceneError> {
    let (scope, _) = dto.clone().resolve(sets, owner, field)?;
    checked(scope, owner, field)
}

fn checked(
    scope: ProcessScope,
    owner: &Owner,
    field: ScopeField,
) -> Result<ProcessScope, SceneError> {
    scope.validate().map_err(|error| SceneError::InvalidScope {
        owner: owner.clone(),
        field,
        error,
    })?;
    Ok(scope)
}

/// 校验并展开全部场景；未被任何绑定引用的场景也会检查，避免启用时才发现错误。
pub fn compile(
    definitions: &SceneDefinitions,
    sets: &AppSets,
) -> Result<CompiledScenes, SceneError> {
    let order = definitions::resolution_order(DefinitionKind::Scene, definitions, |scene| {
        scene.extends.iter().map(String::as_str).collect()
    })?;
    let mut compiled = BTreeMap::new();
    for name in order {
        let owner = Owner::Definition(DefinitionKind::Scene, name.into());
        let dto = &definitions[name];
        let own = Scene {
            foreground: scope(&dto.process_scope, sets, &owner, ScopeField::Process)?,
            pointer: scope(&dto.pointer_scope, sets, &owner, ScopeField::Pointer)?,
        };
        let mut scene = Scene::default();
        for parent in &dto.extends {
            scene = scene
                .intersect(&compiled[parent.as_str()])
                .map_err(|conflict| SceneError::Conflict {
                    owner: owner.clone(),
                    scene: parent.clone(),
                    conflict,
                })?;
        }
        let scene = scene
            .intersect(&own)
            .map_err(|conflict| SceneError::Conflict {
                owner: owner.clone(),
                scene: dto.extends.last().cloned().unwrap_or_else(|| name.into()),
                conflict,
            })?;
        let scene = Scene {
            foreground: checked(scene.foreground, &owner, ScopeField::Process)?,
            pointer: checked(scene.pointer, &owner, ScopeField::Pointer)?,
        };
        compiled.insert(name.to_string(), scene);
    }
    Ok(CompiledScenes(compiled))
}

/// 绑定引用的场景：原文名称、绑定自身条件与读取时的合成结果。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SceneUse {
    name: String,
    own: Scene,
    effective: Scene,
}

impl SceneUse {
    /// 触发描述的上下文仍等于读取时的合成结果时，返回场景名与绑定自身条件；
    /// 上下文已被改写则返回 `None`，按实际范围保存。
    pub fn authored_for(&self, spec: &TriggerSpec) -> Option<(&str, &Scene)> {
        (Scene::of(spec) == self.effective).then_some((self.name.as_str(), &self.own))
    }
}

/// 绑定上下文条件的原文：引用的场景与各范围字段中的集合引用。保存时据此写回原文。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ContextSource {
    pub scene: Option<SceneUse>,
    pub scopes: ScopeSources,
}

impl ContextSource {
    /// 保存或显示时的写法：仍沿用场景时返回场景名与绑定自身条件，否则返回实际上下文。
    pub fn authored(&self, spec: &TriggerSpec) -> (Option<&str>, Scene) {
        match self
            .scene
            .as_ref()
            .and_then(|scene| scene.authored_for(spec))
        {
            Some((name, own)) => (Some(name), own.clone()),
            None => (None, Scene::of(spec)),
        }
    }
}

/// 合成绑定的上下文：引用场景时与绑定自身条件取交集。
pub fn apply(
    scene: Option<String>,
    own: Scene,
    compiled: &CompiledScenes,
    binding: u64,
) -> Result<(Scene, Option<SceneUse>), SceneError> {
    let Some(name) = scene else {
        return Ok((own, None));
    };
    let owner = Owner::Binding(binding);
    let base = definitions::lookup(DefinitionKind::Scene, &compiled.0, &owner, &name)?;
    let effective = base
        .intersect(&own)
        .map_err(|conflict| SceneError::Conflict {
            owner,
            scene: name.clone(),
            conflict,
        })?;
    Ok((
        effective.clone(),
        Some(SceneUse {
            name,
            own,
            effective,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnitrigger_core::{ProcessIdentity, SceneDimension, ScopeIntersectionError};

    fn exe(allow: &[&str], deny: &[&str]) -> ProcessScope {
        ProcessScope {
            identity: ProcessIdentity::Executable,
            allow: allow.iter().map(|s| s.to_string()).collect(),
            deny: deny.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn compile_json(
        sets: serde_json::Value,
        scenes: serde_json::Value,
    ) -> Result<CompiledScenes, SceneError> {
        let sets: AppSets = serde_json::from_value(sets).unwrap();
        crate::app_sets::validate(&sets).unwrap();
        compile(&serde_json::from_value(scenes).unwrap(), &sets)
    }

    #[test]
    fn scenes_resolve_sets_and_compose_by_intersection() {
        let compiled = compile_json(
            serde_json::json!({"browsers": ["chrome", "msedge", "firefox"]}),
            serde_json::json!({
                "browser": {"process_scope": {"allow": ["@browsers"]}, "pointer_scope": {"allow": ["@browsers"]}},
                "chromium-browser": {"extends": ["browser"], "process_scope": {"deny": ["firefox"]}},
                "edge-pointer": {"extends": ["chromium-browser"], "pointer_scope": {"allow": ["msedge"]}}
            }),
        )
        .unwrap();
        assert_eq!(
            compiled.get("edge-pointer").unwrap(),
            &Scene {
                foreground: exe(&["chrome", "msedge", "firefox"], &["firefox"]),
                pointer: exe(&["msedge"], &[]),
            }
        );
    }

    #[test]
    fn conflicts_and_invalid_scopes_name_the_scene() {
        let error = compile_json(
            serde_json::json!({}),
            serde_json::json!({
                "editor": {"process_scope": {"allow": ["code"]}},
                "browser-editor": {"extends": ["editor"], "process_scope": {"allow": ["chrome"]}}
            }),
        )
        .unwrap_err();
        assert_eq!(
            error,
            SceneError::Conflict {
                owner: Owner::Definition(DefinitionKind::Scene, "browser-editor".into()),
                scene: "editor".into(),
                conflict: SceneConflict {
                    dimension: SceneDimension::Foreground,
                    reason: ScopeIntersectionError::DisjointAllow,
                },
            }
        );
        assert_eq!(
            error.to_string(),
            "场景 browser-editor 与场景 editor 的前台应用条件无法合并：两者的应用白名单没有共同名称。"
        );
        let error = compile_json(
            serde_json::json!({}),
            serde_json::json!({"bad": {"pointer_scope": {"allow": ["a/b"]}}}),
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .starts_with("场景 bad 的 pointer_scope：进程名无效"));
        assert_eq!(
            compile_json(
                serde_json::json!({}),
                serde_json::json!({"a": {"extends": ["missing"]}})
            )
            .unwrap_err()
            .to_string(),
            "场景 a 引用了未定义的场景 missing。"
        );
    }

    #[test]
    fn bindings_add_their_own_conditions_and_keep_the_authored_form() {
        let compiled = compile_json(
            serde_json::json!({}),
            serde_json::json!({"browser": {"process_scope": {"allow": ["chrome", "msedge"]}}}),
        )
        .unwrap();
        let own = Scene {
            foreground: exe(&[], &["msedge"]),
            pointer: exe(&["chrome"], &[]),
        };
        let (effective, used) = apply(Some("browser".into()), own.clone(), &compiled, 9).unwrap();
        assert_eq!(
            effective.foreground,
            exe(&["chrome", "msedge"], &["msedge"])
        );
        let mut spec = TriggerSpec {
            pointer_scope: Default::default(),
            wheel: Default::default(),
            keyboard: Default::default(),
            process_scope: Default::default(),
            id: 9,
            held: vec![],
            activation: omnitrigger_core::Activation::Press(omnitrigger_core::Button::Key(75)),
            conditions: vec![],
        };
        effective.apply_to(&mut spec);
        let context = ContextSource {
            scene: used,
            scopes: Default::default(),
        };
        assert_eq!(context.authored(&spec), (Some("browser"), own));
        spec.process_scope = exe(&["notepad"], &[]);
        assert_eq!(context.authored(&spec), (None, Scene::of(&spec)));
        assert_eq!(
            apply(Some("missing".into()), Scene::default(), &compiled, 9)
                .unwrap_err()
                .to_string(),
            "触发器 #9 引用了未定义的场景 missing。"
        );
    }
}
