//! 触发器页设计预览的数据导出：`--export-trigger-preview <文件>`。
//!
//! 预览（`design/`）只负责绘制和交互状态；行摘要、筛选维度、键位布局、
//! 每条触发器点亮的键位与鼠标部位、配色与图标都由这里按应用的同一套代码计算，
//! 避免预览与实际界面各自维护一份规则。

use serde_json::{json, Value};

use crate::settings::Binding;
use crate::trigger_browse::{Facet, GroupBy, Terminal};
use crate::trigger_input_map::{self, MousePart, KEYBOARD};

const FORMAT: u32 = 1;

fn hex(color: u32) -> String {
    format!("#{color:06x}")
}

fn tokens() -> Value {
    let dark: serde_json::Map<_, _> = crate::theme::TOKENS
        .iter()
        .map(|(name, color)| (name.to_string(), json!(hex(*color))))
        .collect();
    let light: serde_json::Map<_, _> = crate::theme::TOKENS
        .iter()
        .map(|(name, color)| {
            (
                name.to_string(),
                json!(hex(crate::theme::light_color(*color))),
            )
        })
        .collect();
    json!({
        "dark": dark,
        "light": light,
        "ui_font": crate::theme::UI_FONT,
        "mono_font": crate::theme::MONO_FONT,
    })
}

fn token_name(color: u32) -> &'static str {
    crate::theme::TOKENS
        .iter()
        .find(|(_, value)| *value == color)
        .map(|(name, _)| *name)
        .unwrap_or("TEXT")
}

fn row(binding: &Binding) -> Value {
    // 预览不连接动作服务，按动作可用处理；不可用原因只在应用内显示。
    let summary = crate::browse_row(binding, true);
    let (glyph, tint) = crate::action_look(&binding.action);
    let (detail, detail_color) = crate::row_detail(binding, None);
    let caps = crate::theme::caps_for(&binding.spec);
    let prefix_len = crate::prefix_caps(&binding.spec).len();
    let key_slots: Vec<u16> = match summary.terminal {
        Terminal::Key(key) => KEYBOARD
            .iter()
            .filter(|slot| trigger_input_map::slot_accepts(slot.vk, key))
            .map(|slot| slot.vk)
            .collect(),
        _ => Vec::new(),
    };
    let held_slots: Vec<u16> = KEYBOARD
        .iter()
        .filter(|slot| {
            summary
                .held_keys
                .iter()
                .any(|&held| trigger_input_map::slot_accepts(slot.vk, held))
        })
        .map(|slot| slot.vk)
        .collect();
    let single = [&summary];
    let mut mouse_terminal = Vec::new();
    let mut mouse_prefix = Vec::new();
    for part in MousePart::ALL {
        let count = trigger_input_map::part_count(&single, part);
        if count.terminal > 0 {
            mouse_terminal.push(part.label());
        }
        if count.prefix > 0 {
            mouse_prefix.push(part.label());
        }
    }
    json!({
        "id": summary.id,
        "name": binding.message,
        "enabled": binding.enabled,
        "icon": glyph.path(),
        "tint": token_name(tint),
        "caps": caps.iter().enumerate().map(|(index, cap)| json!({
            "label": cap.label.as_ref(),
            "mouse": cap.mouse,
            "prefix": index < prefix_len,
            "arrow": cap.arrow.map(|arrow| arrow.path()),
        })).collect::<Vec<_>>(),
        "detail": detail,
        "detail_color": token_name(detail_color),
        "text": summary.text,
        "prefix": summary.prefix,
        "facets": {
            "input": summary.input.label(),
            "action": summary.action.label(),
            "scope": summary.scope.label(),
            "status": summary.status.label(),
        },
        "key_terminal": matches!(summary.terminal, Terminal::Key(_)),
        "key_slots": key_slots,
        "held_slots": held_slots,
        "mouse_terminal": mouse_terminal,
        "mouse_prefix": mouse_prefix,
    })
}

/// 导出预览所需的全部数据。
pub fn export(bindings: &[Binding]) -> Value {
    let facets: Vec<Value> = Facet::ALL
        .into_iter()
        .map(|facet| {
            json!({
                "key": match facet {
                    Facet::Input => "input",
                    Facet::Action => "action",
                    Facet::Scope => "scope",
                    Facet::Status => "status",
                },
                "label": facet.label(),
                "options": facet.options().into_iter().map(|value| value.label()).collect::<Vec<_>>(),
            })
        })
        .collect();
    let group_by: Vec<Value> = GroupBy::ALL
        .into_iter()
        .map(|group| {
            json!({
                "key": match group {
                    GroupBy::Prefix => "prefix",
                    GroupBy::Action => "action",
                    GroupBy::Scope => "scope",
                    GroupBy::Input => "input",
                    GroupBy::None => "none",
                },
                "label": group.label(),
            })
        })
        .collect();
    let icons: serde_json::Map<_, _> = crate::assets::icon_svgs()
        .map(|(path, svg)| (path.to_string(), json!(svg)))
        .collect();
    json!({
        "format": FORMAT,
        "tokens": tokens(),
        "icons": icons,
        "facets": facets,
        "group_by": group_by,
        "keyboard": {
            "width": trigger_input_map::KEYBOARD_WIDTH,
            "height": trigger_input_map::KEYBOARD_HEIGHT,
            "slots": KEYBOARD.iter().map(|slot| json!({
                "vk": slot.vk, "label": slot.label, "x": slot.x, "y": slot.y, "w": slot.w,
            })).collect::<Vec<_>>(),
        },
        "mouse_parts": MousePart::ALL.iter().map(|part| part.label()).collect::<Vec<_>>(),
        "rows": bindings.iter().map(row).collect::<Vec<_>>(),
    })
}

/// 读取当前配置并写出预览数据；与 `--config`、`--profile` 组合使用。
pub fn cli(arguments: &[String]) -> Option<Result<String, String>> {
    let index = arguments
        .iter()
        .position(|argument| argument == "--export-trigger-preview")?;
    Some((|| {
        let path = arguments
            .get(index + 1)
            .filter(|path| !path.starts_with("--"))
            .ok_or("--export-trigger-preview 后需要输出文件路径。")?;
        let bindings = crate::settings::load()?;
        let text = serde_json::to_string_pretty(&export(&bindings)).map_err(|e| e.to_string())?;
        std::fs::write(path, text + "\n").map_err(|error| format!("写入 {path} 失败：{error}"))?;
        Ok(format!(
            "已导出 {} 条触发器的预览数据：{path}（配置 {}）",
            bindings.len(),
            crate::settings::path().display()
        ))
    })())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_export_lights_the_same_keys_as_the_keyboard_view() {
        let bindings = crate::settings::decode_for_tests(include_str!(
            "../../../tests/fixtures/profiles/ui-browse/bindings.json"
        ));
        let value = export(&bindings);
        let rows = value["rows"].as_array().unwrap();
        assert_eq!(rows.len(), bindings.len());
        // 预览按行数据汇总出的键位，应与应用键盘视图在“全部层”下标出的一致。
        let summaries: Vec<_> = bindings
            .iter()
            .map(|binding| crate::browse_row(binding, true))
            .collect();
        let refs: Vec<_> = summaries.iter().collect();
        let view = trigger_input_map::keyboard_view(&refs, None, None);
        let mut exported: Vec<u16> = rows
            .iter()
            .flat_map(|row| row["key_slots"].as_array().unwrap().clone())
            .map(|vk| vk.as_u64().unwrap() as u16)
            .collect();
        exported.sort_unstable();
        exported.dedup();
        let mut bound = view.bound.clone();
        bound.sort_unstable();
        assert_eq!(exported, bound);
        let capslock = rows.iter().find(|row| row["prefix"] == "CapsLock").unwrap();
        assert_eq!(capslock["held_slots"], json!([0x14]));
        assert!(value["icons"][rows[0]["icon"].as_str().unwrap()].is_string());
        assert!(value["tokens"]["dark"]["ACCENT"].is_string());
    }

    #[test]
    fn committed_design_data_matches_the_app_export() {
        let bindings = crate::settings::decode_for_tests(include_str!(
            "../../../tests/fixtures/profiles/ui-browse/bindings.json"
        ));
        let committed: Value =
            serde_json::from_str(include_str!("../../../design/data/ui-browse.json")).unwrap();
        assert!(
            committed == export(&bindings),
            "design/data/ui-browse.json 已过期：在 design 目录运行 npm run export 重新导出。"
        );
    }
}
