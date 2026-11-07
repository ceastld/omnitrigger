//! 当前动作版本提示的有界参数与可信调用作用域；不创建UI或执行平台操作。
use crate::{ActionCall, ActionError};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const METHOD: &str = "ui.action_hint";
/// 只供服务内层转交，不能加入脚本桥接的方法白名单。
pub const SCOPED_METHOD: &str = "ui.action_hint.scoped";

pub fn fault(code: &str, effect: &str) -> ActionError {
    let message = match code {
        "invalid_input" => {
            "动作版本提示需要1–16个UTF-16单位的单行非空文字，且不能带调用身份或样式参数。"
        }
        "ui_hint_scope_expired" => "本次动作调用已结束或身份不匹配，版本提示未更新。",
        "ui_busy" => "界面请求通道正忙，版本提示未开始更新。",
        "cancelled" => "动作版本提示更新已取消。",
        "timeout" => "动作版本提示更新未在时限内确认。",
        _ => "动作版本提示需要本次调用所属的OmniTrigger界面。",
    };
    let mut error = ActionError::new(code, message);
    error.method = Some(METHOD.into());
    error.effect_state = Some(effect.into());
    error
}

pub fn validate_text(text: &str) -> Result<(), ActionError> {
    if text.is_empty()
        || text.trim() != text
        || text.encode_utf16().count() > 16
        || text.chars().any(char::is_control)
        || text.contains(['\u{2028}', '\u{2029}', '\u{feff}'])
    {
        return Err(fault("invalid_input", "not_started"));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    text: String,
}
pub(crate) fn options(params: Value) -> Result<String, ActionError> {
    let value: Options =
        serde_json::from_value(params).map_err(|_| fault("invalid_input", "not_started"))?;
    validate_text(&value.text)?;
    Ok(value.text)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionHintScope {
    pub binding_id: u64,
    pub invocation_id: u64,
    pub package_id: String,
    pub action_id: String,
    pub action_version: u32,
}
impl ActionHintScope {
    pub fn matches_call(&self, call: &ActionCall) -> bool {
        self.package_id == call.package_id
            && self.action_id == call.action_id
            && self.action_version == call.action_version
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopedActionHint {
    pub scope: ActionHintScope,
    pub text: String,
}
impl ScopedActionHint {
    pub fn parse(params: Value) -> Result<Self, ActionError> {
        let value: Self =
            serde_json::from_value(params).map_err(|_| fault("invalid_input", "not_started"))?;
        if value.scope.invocation_id == 0
            || value.scope.action_version == 0
            || !crate::model::valid_id(&value.scope.package_id)
            || !crate::model::valid_id(&value.scope.action_id)
        {
            return Err(fault("invalid_input", "not_started"));
        }
        validate_text(&value.text)?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn action_hint_text_is_bounded_by_utf16_and_never_normalized() {
        for text in [
            "",
            " n10",
            "n10 ",
            "n10\n",
            "a\u{0085}b",
            "a\u{2028}b",
            "\u{feff}n10",
        ] {
            assert_eq!(validate_text(text).unwrap_err().code, "invalid_input");
        }
        assert!(validate_text(&"🙂".repeat(8)).is_ok());
        assert!(validate_text(&"🙂".repeat(9)).is_err());
    }
    #[test]
    fn public_action_hint_options_cannot_select_identity_or_style() {
        for value in [
            json!(null),
            json!({}),
            json!({"text":"n10","scope":{}}),
            json!({"text":"n10","color":"green"}),
        ] {
            assert_eq!(options(value).unwrap_err().code, "invalid_input");
        }
        assert_eq!(options(json!({"text":"pre"})).unwrap(), "pre");
    }
    #[test]
    fn scoped_action_hint_validates_wire_identity_and_text() {
        let scope = json!({"binding_id":0,"invocation_id":u64::MAX,"package_id":"test","action_id":"run","action_version":3});
        let value = ScopedActionHint::parse(json!({"scope":scope,"text":"n10"})).unwrap();
        assert_eq!(value.scope.invocation_id, u64::MAX);
        assert_eq!(value.scope.binding_id, 0);
        let mut invalid = json!({"scope":scope,"text":"n10"});
        invalid["scope"]["action_version"] = json!(0);
        assert_eq!(
            ScopedActionHint::parse(invalid).unwrap_err().code,
            "invalid_input"
        );
    }
    #[test]
    fn scoped_hint_definition_match_checks_version_but_not_business_input() {
        let scope = ActionHintScope {
            binding_id: 1,
            invocation_id: 2,
            package_id: "test".into(),
            action_id: "run".into(),
            action_version: 3,
        };
        let mut call = ActionCall {
            package_id: "test".into(),
            action_id: "run".into(),
            action_version: 3,
            input: json!({"secret":"not forwarded"}),
        };
        assert!(scope.matches_call(&call));
        call.action_version = 4;
        assert!(!scope.matches_call(&call));
    }
}
