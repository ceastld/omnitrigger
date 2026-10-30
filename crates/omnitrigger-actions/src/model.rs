use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

pub const MAX_INPUT_BYTES: usize = 64 * 1024;

/// 稳定的持久化调用信封；包是否安装与参数业务含义由目录和运行时分别判断。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionCall {
    pub package_id: String,
    pub action_id: String,
    pub action_version: u32,
    #[serde(default = "empty_input")]
    pub input: Value,
}

fn empty_input() -> Value {
    Value::Object(Default::default())
}

impl ActionCall {
    pub fn validate(&self) -> Result<(), ActionError> {
        if !valid_id(&self.package_id) || !valid_id(&self.action_id) || self.action_version == 0 {
            return Err(ActionError::new(
                "invalid_reference",
                "动作包、动作标识或契约版本无效。",
            ));
        }
        if !self.input.is_object() {
            return Err(ActionError::new(
                "invalid_input",
                "动作输入必须是 JSON 对象。",
            ));
        }
        if serde_json::to_vec(&self.input)
            .map_err(|error| ActionError::new("invalid_input", error.to_string()))?
            .len()
            > MAX_INPUT_BYTES
        {
            return Err(ActionError::new(
                "input_too_large",
                "动作输入不能超过 64 KiB。",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageManifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub actions: Vec<ActionDefinition>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionDefinition {
    pub id: String,
    pub title: String,
    pub version: u32,
    pub runtime: String,
    pub entry: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionError {
    pub code: String,
    pub message: String,
}

impl ActionError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ActionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}（{}）", self.message, self.code)
    }
}

impl std::error::Error for ActionError {}

pub(crate) fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn references_remain_valid_without_an_installed_provider() {
        let call: ActionCall = serde_json::from_value(json!({
            "package_id":"missing.provider", "action_id":"future", "action_version":7
        }))
        .unwrap();
        call.validate().unwrap();
        assert_eq!(call.input, json!({}));
        assert_eq!(
            serde_json::from_str::<ActionCall>(&serde_json::to_string(&call).unwrap()).unwrap(),
            call
        );
    }

    #[test]
    fn invalid_envelopes_fail_without_interpreting_business_input() {
        let mut call = ActionCall {
            package_id: "example".into(),
            action_id: "run".into(),
            action_version: 1,
            input: json!({"anything":{"is":"preserved"}}),
        };
        call.validate().unwrap();
        call.input = json!([]);
        assert_eq!(call.validate().unwrap_err().code, "invalid_input");
        call.input = json!({"large":"x".repeat(MAX_INPUT_BYTES)});
        assert_eq!(call.validate().unwrap_err().code, "input_too_large");
        call.input = json!({});
        for invalid in ["", "..", "a/b", "a\\b"] {
            call.package_id = invalid.into();
            assert_eq!(call.validate().unwrap_err().code, "invalid_reference");
        }
        assert!(serde_json::from_value::<ActionCall>(
            json!({"package_id":"a","action_id":"b","action_version":1,"unexpected":true})
        )
        .is_err());
    }
}
