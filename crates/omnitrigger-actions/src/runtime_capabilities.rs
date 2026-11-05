//! 只比较明确提供的宿主描述与能力要求，不启动进程、不读取工程、不推断源码用法。
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;

pub const MAX_DESCRIPTION_BYTES: usize = 4096;
const MAX_CAPABILITIES: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeDescription {
    pub version: String,
    pub engine: String,
    /// None 表示旧宿主未声明；Some([]) 表示明确声明空能力集。
    pub capabilities: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CapabilityError {
    pub code: String,
    pub message: String,
    pub effect_state: String,
    pub missing_capabilities: Vec<String>,
}

fn fault(code: &str, message: impl Into<String>, missing: Vec<String>) -> CapabilityError {
    CapabilityError {
        code: code.into(),
        message: message.into(),
        effect_state: "not_started".into(),
        missing_capabilities: missing,
    }
}

fn valid_capability(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}

/// 能力名是精确契约标识，不以版本号、前缀或 SDK 文件内容推断支持。
pub fn validate_requirements(required: &[String]) -> Result<(), CapabilityError> {
    let mut unique = BTreeSet::new();
    if required.len() > MAX_CAPABILITIES
        || required
            .iter()
            .any(|s| !valid_capability(s) || !unique.insert(s))
    {
        return Err(fault(
            "invalid_runtime_requirements",
            "能力要求必须是至多 32 个不重复的有效标识。",
            vec![],
        ));
    }
    Ok(())
}

impl RuntimeDescription {
    pub fn parse(bytes: &[u8]) -> Result<Self, CapabilityError> {
        let invalid = || {
            fault(
                "runtime_description_invalid",
                "宿主能力描述无效或协议不兼容。",
                vec![],
            )
        };
        if bytes.len() > MAX_DESCRIPTION_BYTES {
            return Err(invalid());
        }
        let value: Value = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        let object = value.as_object().ok_or_else(invalid)?;
        if value["schema_version"] != 1
            || value["product"] != "omnitrigger-script-host"
            || value["api_version"] != 1
            || value["protocol_schema"] != 1
            || !matches!(value["engine"].as_str(), Some("boa" | "quickjs"))
        {
            return Err(invalid());
        }
        let version = value["version"].as_str().ok_or_else(invalid)?;
        if version.is_empty()
            || version.len() > 64
            || !version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
        {
            return Err(invalid());
        }
        let capabilities = match object.get("capabilities") {
            None => None,
            Some(list) => {
                let list = list.as_array().ok_or_else(invalid)?;
                if list.len() > MAX_CAPABILITIES {
                    return Err(invalid());
                }
                let mut names = Vec::new();
                for name in list {
                    names.push(name.as_str().ok_or_else(invalid)?.to_owned());
                }
                validate_requirements(&names).map_err(|_| invalid())?;
                Some(names)
            }
        };
        Ok(Self {
            version: version.to_owned(),
            engine: value["engine"].as_str().ok_or_else(invalid)?.to_owned(),
            capabilities,
        })
    }

    /// 未提出要求的旧动作照常可用；未声明能力的旧宿主不被猜成支持新能力。
    /// 支持只表示接口契约存在，不代表权限、原生桥接或设备当前可用。
    pub fn check(&self, required: &[String]) -> Result<(), CapabilityError> {
        validate_requirements(required)?;
        if required.is_empty() {
            return Ok(());
        }
        let Some(available) = &self.capabilities else {
            return Err(fault(
                "runtime_capabilities_unknown",
                "宿主未声明所需能力，请更新脚本环境后重试。",
                required.to_vec(),
            ));
        };
        let missing: Vec<_> = required
            .iter()
            .filter(|name| !available.contains(name))
            .cloned()
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        Err(fault(
            "runtime_capability_missing",
            format!(
                "脚本环境缺少能力：{}。请更新脚本环境后重试。",
                missing.join("、")
            ),
            missing,
        ))
    }
}
