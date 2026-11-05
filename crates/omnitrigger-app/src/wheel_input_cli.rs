//! 仅为本次监听选择滚轮输入策略；不读写配置，不安装 Hook，不更改旧 CLI 默认行为。
use omnitrigger_actions::ActionError;
use omnitrigger_core::WheelInputPolicy;
use std::collections::HashSet;

const FLAG: &str = "--wheel-input-policy";

fn invalid(message: &str) -> ActionError {
    let mut error = ActionError::new("invalid_cli_arguments", message);
    error.effect_state = Some("not_started".into());
    error
}

/// 在所有 CLI 分发及副作用之前调用。既接受完整 argv，也接受以选项开头的切片。
/// 没有本选项时直接采用原策略，不收紧已有命令的解析和兼容行为。
pub fn parse(arguments: &[String]) -> Result<WheelInputPolicy, ActionError> {
    let start = usize::from(arguments.first().is_some_and(|arg| !arg.starts_with("--")));
    let arguments = &arguments[start..];
    if !arguments
        .iter()
        .any(|arg| arg == FLAG || arg.starts_with("--wheel-input-policy="))
    {
        return Ok(WheelInputPolicy::Unfiltered);
    }
    let mut seen = HashSet::new();
    let mut policy = None;
    let mut index = 0;
    while index < arguments.len() {
        let flag = arguments[index].as_str();
        if !seen.insert(flag) {
            return Err(invalid("显式滚轮策略启动参数不能重复。"));
        }
        match flag {
            FLAG => {
                let value = arguments.get(index + 1)
                    .filter(|value| !value.starts_with("--"))
                    .ok_or_else(|| invalid("--wheel-input-policy 后需要 unfiltered 或 small-vertical-150ms。"))?;
                policy = Some(match value.as_str() {
                    "unfiltered" => WheelInputPolicy::Unfiltered,
                    "small-vertical-150ms" => WheelInputPolicy::SmallVertical150ms,
                    _ => return Err(invalid("滚轮输入策略仅支持 unfiltered 或 small-vertical-150ms。")),
                });
                index += 2;
            }
            "--config" | "--wheel" => {
                arguments.get(index + 1)
                    .filter(|value| !value.is_empty() && !value.starts_with("--"))
                    .ok_or_else(|| invalid("显式滚轮策略启动中的 --config 或 --wheel 缺少参数。"))?;
                index += 2;
            }
            "--smoke-test" | "--ui-smoke" | "--settings" | "--browser-connector-settings"
            | "--panel" | "--wheel-editor" => index += 1,
            _ => return Err(invalid("--wheel-input-policy 仅可用于普通界面启动或 --smoke-test，不能与无 Hook 操作或其他参数混用。")),
        }
    }
    policy.ok_or_else(|| invalid("--wheel-input-policy 需要独立参数和值，不支持等号形式。"))
}

#[cfg(test)]
#[path = "../../../tests/wheel_input_cli.rs"]
mod tests;
