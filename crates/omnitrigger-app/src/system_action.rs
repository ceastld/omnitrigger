//! 系统调节动作的持久化参数；设备访问只在后台执行器中发生。
use crate::theme::rgb;
use crate::{caption, section_title, small, theme, ActionSpec, Workspace};
use gpui::{div, prelude::*, App, Context, Div};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdjustmentTarget {
    Volume,
    Brightness,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemAdjustmentAction {
    pub target: AdjustmentTarget,
    pub delta_percent: i16,
}
impl SystemAdjustmentAction {
    pub fn platform(self) -> omnitrigger_windows::SystemAdjustment {
        match self.target {
            AdjustmentTarget::Volume => omnitrigger_windows::SystemAdjustment::Volume {
                delta_percent: self.delta_percent,
            },
            AdjustmentTarget::Brightness => omnitrigger_windows::SystemAdjustment::Brightness {
                delta_percent: self.delta_percent,
            },
        }
    }
    pub fn validate(self) -> Result<(), String> {
        omnitrigger_windows::validate_system_adjustment(self.platform())
            .map_err(|error| error.to_string())
    }
    pub fn display(self) -> String {
        format!(
            "{}{} {}%",
            match self.target {
                AdjustmentTarget::Volume => "音量",
                AdjustmentTarget::Brightness => "亮度",
            },
            if self.delta_percent > 0 {
                "增加"
            } else {
                "减少"
            },
            self.delta_percent.unsigned_abs()
        )
    }
}

/// 显式系统命令不创建界面或安装 Hook；检查命令只读取设备能力。
pub fn cli(arguments: &[String]) -> Option<Result<String, String>> {
    let operations = [
        "--check-system-controls",
        "--adjust-volume",
        "--adjust-brightness",
    ];
    let chosen: Vec<_> = operations
        .into_iter()
        .filter_map(|flag| {
            arguments
                .iter()
                .position(|argument| argument == flag)
                .map(|index| (flag, index))
        })
        .collect();
    if chosen.is_empty() {
        return None;
    }
    Some((|| {
        if chosen.len() != 1
            || arguments
                .iter()
                .filter(|argument| operations.contains(&argument.as_str()))
                .count()
                != 1
            || arguments.iter().any(|argument| {
                matches!(
                    argument.as_str(),
                    "--check-config" | "--smoke-test" | "--wheel" | "--wheel-editor" | "--panel"
                )
            })
        {
            return Err("系统调节命令不能与其他检查或界面启动命令混用。".into());
        }
        let point = if let Some(index) = arguments.iter().position(|argument| argument == "--point")
        {
            let value = arguments
                .get(index + 1)
                .ok_or("--point 后需要 x,y 物理坐标。")?;
            let (x, y) = value.split_once(',').ok_or("坐标格式为 x,y。")?;
            (
                x.trim().parse::<i32>().map_err(|_| "横坐标无效。")?,
                y.trim().parse::<i32>().map_err(|_| "纵坐标无效。")?,
            )
        } else {
            omnitrigger_windows::capture_adjustment_point().map_err(|error| error.to_string())?
        };
        let (flag, index) = chosen[0];
        if flag == "--check-system-controls" {
            let capabilities = omnitrigger_windows::query_system_adjustment_capabilities_at(point);
            let encode = |capability: omnitrigger_windows::AdjustmentCapability| serde_json::json!({"available":capability.available,"current_percent":capability.current_percent,"target":capability.target_label,"method":capability.brightness_method.map(|method|format!("{method:?}")),"reason":capability.reason});
            return serde_json::to_string_pretty(&serde_json::json!({"point":[point.0,point.1],"volume":encode(capabilities.volume),"brightness":encode(capabilities.brightness)})).map_err(|error|error.to_string());
        }
        let delta_percent = arguments
            .get(index + 1)
            .ok_or("调节命令后需要非零百分点，例如 5 或 -5。")?
            .parse::<i16>()
            .map_err(|_| "调节幅度需要 -100 至 100 的非零整数。")?;
        let action = SystemAdjustmentAction {
            target: if flag == "--adjust-volume" {
                AdjustmentTarget::Volume
            } else {
                AdjustmentTarget::Brightness
            },
            delta_percent,
        };
        action.validate()?;
        let result = omnitrigger_windows::adjust_system_at(action.platform(), point)
            .map_err(|error| error.to_string())?;
        Ok(format!(
            "{} · 当前 {}% · {}",
            action.display(),
            result.current_percent,
            result.target_label
        ))
    })())
}

impl Workspace {
    pub(super) fn draft_adjustment(&self, cx: &App) -> Result<SystemAdjustmentAction, String> {
        let ActionSpec::AdjustSystem(action) = self.draft_action else {
            return Err("未选择系统调节动作。".into());
        };
        let amount = self
            .adjustment_amount
            .read(cx)
            .text()
            .trim()
            .parse::<i16>()
            .map_err(|_| "调节幅度应为 1–100 的整数百分比。")?;
        if !(1..=100).contains(&amount) {
            return Err("调节幅度应为 1–100%。".into());
        }
        let action = SystemAdjustmentAction {
            delta_percent: if action.delta_percent > 0 {
                amount
            } else {
                -amount
            },
            ..action
        };
        action.validate()?;
        Ok(action)
    }
    pub(super) fn system_action_fields(&self, cx: &mut Context<Self>) -> Div {
        let ActionSpec::AdjustSystem(action) = self.draft_action else {
            return div();
        };
        let mut options = div().grid().grid_cols(2).gap(gpui::px(6.));
        for (index, target, positive, label) in [
            (0, AdjustmentTarget::Volume, true, "音量增加"),
            (1, AdjustmentTarget::Volume, false, "音量减少"),
            (2, AdjustmentTarget::Brightness, true, "亮度增加"),
            (3, AdjustmentTarget::Brightness, false, "亮度减少"),
        ] {
            let active = action.target == target && (action.delta_percent > 0) == positive;
            options = options.child(
                small(crate::button(
                    ("adjust-system-mode", index as u64),
                    label,
                    if active {
                        crate::Tone::Primary
                    } else {
                        crate::Tone::Secondary
                    },
                    true,
                ))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.draft_action = ActionSpec::AdjustSystem(SystemAdjustmentAction {
                        target,
                        delta_percent: if positive { 5 } else { -5 },
                    });
                    cx.notify();
                })),
            );
        }
        div().flex().flex_col().gap(gpui::px(9.)).child(section_title("系统调节")).child(options)
            .child(Self::input_field("每次触发调节（%）",&self.adjustment_amount,Some("1–100；滚轮每累计一格执行一次，自动限制在设备支持的范围内。")))
            .child(caption(match action.target {AdjustmentTarget::Volume=>"按百分点精确调节音量，不显示音量浮层。需要 Windows 自带提示时，选择“发送按键”并填写 VolumeUp 或 VolumeDown；步长由系统决定。",AdjustmentTarget::Brightness=>"调节触发时鼠标所在的屏幕；使用硬件 DDC/CI 或已匹配的内屏亮度接口。不支持的屏幕会报告原因。"}))
            .child(caption("“测试动作”会真实改变音量或亮度；仅检查支持情况可使用 --check-system-controls。"))
            .when_some(self.draft_adjustment(cx).err(),|el,error|el.child(div().text_color(rgb(theme::DANGER)).text_size(gpui::px(11.)).child(error)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_or_ambiguous_commands_never_reach_hardware() {
        for args in [
            vec!["app", "--adjust-volume", "0", "--point", "0,0"],
            vec!["app", "--adjust-brightness", "101", "--point", "0,0"],
            vec!["app", "--adjust-volume", "5", "--check-config"],
            vec!["app", "--adjust-volume", "5", "--adjust-volume", "10"],
            vec!["app", "--check-system-controls", "--point", "invalid"],
        ] {
            let arguments = args.into_iter().map(String::from).collect::<Vec<_>>();
            assert!(cli(&arguments).unwrap().is_err());
        }
        assert!(cli(&["app".into(), "--check-config".into()]).is_none());
    }
}
