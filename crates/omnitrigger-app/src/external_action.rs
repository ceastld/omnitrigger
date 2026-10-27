//! 外部动作显式保存依赖及命令；其进程退出不代表外部流程已经完成。
use crate::execution::CommandAction;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalAction {
    pub provider: String,
    pub command: CommandAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
}

impl ExternalAction {
    pub fn validate(&self) -> Result<(), String> {
        if self.provider.trim().is_empty()
            || self.provider.chars().count() > 30
            || self.provider.chars().any(char::is_control)
        {
            return Err("外部动作依赖名称需要 1–30 个字符。".into());
        }
        if !std::path::Path::new(&self.command.program).is_absolute() {
            return Err("外部动作需要使用依赖程序的绝对路径。".into());
        }
        if self.disabled_reason.as_ref().is_some_and(|reason| {
            reason.trim().is_empty() || reason.chars().count() > 2000 || reason.contains('\0')
        }) {
            return Err("不可用原因需要 1–2000 个字符，不能包含空字符。".into());
        }
        self.command.validate()
    }

    pub fn unavailable_reason(&self) -> Option<String> {
        if let Some(reason) = &self.disabled_reason {
            return Some(reason.clone());
        }
        (!std::path::Path::new(&self.command.program).is_file())
            .then(|| format!("{} 依赖程序不存在：{}", self.provider, self.command.program))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_external_action_stays_unavailable_even_when_its_launcher_exists() {
        let action = ExternalAction {
            provider: "外部工具".into(),
            command: CommandAction {
                program: std::env::current_exe()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                args: vec!["动作标识".into()],
                working_directory: None,
            },
            disabled_reason: Some("源动作不存在".into()),
        };
        action.validate().unwrap();
        assert_eq!(action.unavailable_reason().as_deref(), Some("源动作不存在"));
        let mut restored = action.clone();
        restored.disabled_reason = None;
        assert_eq!(restored.unavailable_reason(), None);
        let mut value = serde_json::to_value(action).unwrap();
        value["target"] = "other".into();
        assert!(serde_json::from_value::<ExternalAction>(value).is_err());
        restored.command.program = "launcher.exe".into();
        assert!(restored.validate().is_err());
    }
}
