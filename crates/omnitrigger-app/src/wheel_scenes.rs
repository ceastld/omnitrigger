//! 轮盘槽位的场景引用：只有部分槽位需要按应用区分时，给这些槽位引用一条场景规则，
//! 不必为每个应用复制整组轮盘。
//!
//! 场景定义在 bindings.json，与触发器共用同一套匹配规则。轮盘只按目标窗口（触发时的前台窗口）
//! 判断，因此只能引用不限制指针窗口的场景。同一方位上，命中的专属槽位按范围具体程度
//! 盖过通用槽位；具体程度相同且范围重叠的两个槽位在校验时报错，不按书写顺序暗中取舍。
use crate::scenes::CompiledScenes;
use crate::wheels::{WheelDefinition, WheelSlot};
use omnitrigger_core::{ProcessScope, WindowRole};
use std::collections::HashMap;

/// 目标窗口的进程映像名与窗口角色；读取失败时进程为 `None`，场景槽位均不显示。
#[derive(Clone, Copy, Debug)]
pub struct TargetContext<'a> {
    pub process: Option<&'a str>,
    pub role: WindowRole,
}

/// 槽位可引用的场景范围：只取前台应用条件，场景缺失或限制了指针窗口时报错。
pub fn scene_scope(scenes: &CompiledScenes, name: &str) -> Result<ProcessScope, String> {
    let scene = scenes
        .get(name)
        .ok_or_else(|| format!("引用了未定义的场景 {name}。"))?;
    if !scene.pointer.is_global() {
        return Err(format!(
            "引用的场景 {name} 限制了指针窗口；轮盘只按目标窗口判断，请改用只限定前台应用的场景。"
        ));
    }
    scene
        .foreground
        .normalized()
        .map_err(|error| error.to_string())
}

/// 用于编辑基线比较：同一匹配域里，名单顺序和不影响白名单的排除项不改变可见槽位。
pub(super) fn effective_scene_scope(
    scenes: &CompiledScenes,
    name: &str,
) -> Result<ProcessScope, String> {
    let mut scope = scene_scope(scenes, name)?;
    if scope.is_global() {
        return Ok(ProcessScope::default());
    }
    if scope.allow.is_empty() {
        scope.deny.sort_unstable();
    } else {
        scope.allow.retain(|name| !scope.deny.contains(name));
        scope.allow.sort_unstable();
        scope.deny.clear();
    }
    Ok(scope)
}

/// 校验一组槽位的场景引用：场景存在、不限制指针窗口，且同一方位的槽位不会同时以相同具体程度命中。
pub fn check_slots(slots: &[WheelSlot], scenes: &CompiledScenes) -> Result<(), String> {
    let mut by_position: HashMap<(u8, u8), Vec<(usize, ProcessScope)>> = HashMap::new();
    for (index, slot) in slots.iter().enumerate() {
        let scope = match &slot.scene {
            Some(name) => {
                scene_scope(scenes, name).map_err(|error| format!("槽位 {}：{error}", index + 1))?
            }
            None => ProcessScope::default(),
        };
        if let Some(position) = slot.position {
            by_position
                .entry((position.ring, position.sector))
                .or_default()
                .push((index, scope));
        }
    }
    for entries in by_position.values() {
        for (offset, (first, left)) in entries.iter().enumerate() {
            for (second, right) in &entries[offset + 1..] {
                if left.specificity() == right.specificity() && left.intersects(right) {
                    return Err(format!(
                        "槽位 {} 与槽位 {} 位于同一方位，且场景范围有重叠、具体程度相同；请调整场景或移开其中一个。",
                        first + 1,
                        second + 1
                    ));
                }
            }
        }
    }
    Ok(())
}

/// 校验轮盘中所有槽位（含旧版应用覆盖）的场景引用。
pub fn check_wheel(wheel: &WheelDefinition, scenes: &CompiledScenes) -> Result<(), String> {
    check_slots(&wheel.slots, scenes)?;
    for application in &wheel.app_overrides {
        check_slots(&application.slots, scenes)
            .map_err(|error| format!("应用“{}”：{error}", application.process))?;
    }
    Ok(())
}

/// 打开轮盘时的完整槽位快照：先按旧版应用覆盖选组，再按场景筛选槽位。
/// `legacy_process` 是旧版覆盖使用的上下文（桌面为 `@desktop`）。
pub fn resolve_for_target(
    wheel: &WheelDefinition,
    scenes: &CompiledScenes,
    legacy_process: Option<&str>,
    context: TargetContext<'_>,
) -> WheelDefinition {
    let mut resolved = wheel.resolved_for_process(legacy_process);
    resolved.slots = resolve_slots(&resolved.slots, scenes, context);
    resolved
}

/// 目标上下文下实际显示的槽位：去掉不命中的场景槽位，同一方位保留范围最具体的一个。
/// 场景缺失或无效的槽位不显示；校验阶段会报告原因。
pub fn resolve_slots(
    slots: &[WheelSlot],
    scenes: &CompiledScenes,
    context: TargetContext<'_>,
) -> Vec<WheelSlot> {
    let mut winners: HashMap<(u8, u8), (u8, usize)> = HashMap::new();
    let mut visible = Vec::new();
    for (index, slot) in slots.iter().enumerate() {
        let specificity = match &slot.scene {
            None => 0,
            Some(name) => match scene_scope(scenes, name) {
                Ok(scope) if scope.matches(context.process, context.role) => scope.specificity(),
                _ => continue,
            },
        };
        match slot.position {
            Some(position) => {
                let key = (position.ring, position.sector);
                match winners.get(&key) {
                    Some(&(best, _)) if best >= specificity => {}
                    _ => {
                        winners.insert(key, (specificity, index));
                    }
                }
            }
            None => visible.push(index),
        }
    }
    visible.extend(winners.into_values().map(|(_, index)| index));
    visible.sort_unstable();
    visible
        .into_iter()
        .map(|index| WheelSlot {
            scene: None,
            ..slots[index].clone()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_sets::AppSets;
    use crate::settings::ActionSpec;
    use crate::wheels::WheelPosition;

    fn scenes(json: serde_json::Value) -> CompiledScenes {
        crate::scenes::compile(&serde_json::from_value(json).unwrap(), &AppSets::new()).unwrap()
    }

    fn slot(label: &str, sector: u8, scene: Option<&str>) -> WheelSlot {
        WheelSlot {
            label: label.into(),
            action: ActionSpec::Notification,
            position: Some(WheelPosition {
                ring: 0,
                sector,
                sectors: 8,
                half_step: false,
            }),
            scene: scene.map(Into::into),
        }
    }

    fn labels(slots: &[WheelSlot]) -> Vec<&str> {
        slots.iter().map(|slot| slot.label.as_str()).collect()
    }

    fn library() -> CompiledScenes {
        scenes(serde_json::json!({
            "chrome": {"process_scope": {"allow": ["chrome"]}},
            "editors": {"process_scope": {"allow": ["code", "devenv"]}},
            "not-desktop": {"process_scope": {"identity": "shell_context", "deny": ["desktop"]}},
            "browser-pointer": {"pointer_scope": {"allow": ["chrome"]}}
        }))
    }

    #[test]
    fn scene_slots_cover_shared_slots_only_where_they_match() {
        let slots = vec![
            slot("关闭标签", 1, None),
            slot("关闭网页", 1, Some("chrome")),
            slot("重新打开", 2, Some("editors")),
            slot("后退", 3, Some("not-desktop")),
        ];
        let scenes = library();
        check_slots(&slots, &scenes).unwrap();
        let at = |process, role| {
            resolve_slots(
                &slots,
                &scenes,
                TargetContext {
                    process: Some(process),
                    role,
                },
            )
        };
        assert_eq!(
            labels(&at("chrome.exe", WindowRole::Application)),
            ["关闭网页", "后退"]
        );
        assert_eq!(
            labels(&at("Code.exe", WindowRole::Application)),
            ["关闭标签", "重新打开", "后退"]
        );
        assert_eq!(
            labels(&at("explorer.exe", WindowRole::Desktop)),
            ["关闭标签"]
        );
        assert!(at("chrome.exe", WindowRole::Application)
            .iter()
            .all(|slot| slot.scene.is_none()));
        let unknown = resolve_slots(
            &slots,
            &scenes,
            TargetContext {
                process: None,
                role: WindowRole::Unknown,
            },
        );
        assert_eq!(labels(&unknown), ["关闭标签"]);
    }

    #[test]
    fn ambiguous_unknown_and_pointer_scenes_are_rejected() {
        let scenes = library();
        let error = check_slots(
            &[
                slot("a", 1, Some("chrome")),
                slot("b", 1, Some("editors")),
                slot("c", 1, Some("chrome")),
            ],
            &scenes,
        )
        .unwrap_err();
        assert!(error.contains("槽位 1 与槽位 3"), "{error}");
        let error = check_slots(
            &[slot("a", 1, None), slot("b", 1, Some("not-desktop"))],
            &scenes,
        )
        .unwrap_err();
        assert!(error.contains("具体程度相同"), "{error}");
        assert_eq!(
            check_slots(&[slot("a", 1, Some("missing"))], &scenes).unwrap_err(),
            "槽位 1：引用了未定义的场景 missing。"
        );
        assert!(
            check_slots(&[slot("a", 1, Some("browser-pointer"))], &scenes)
                .unwrap_err()
                .contains("限制了指针窗口")
        );
    }
}
