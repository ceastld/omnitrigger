//! RaiShot 文件完成协议。仅处理小回执；不执行进程或读取图像。
use serde::Deserialize;

pub const MAX_BYTES: u64 = 67_108_864;
pub const MAX_PIXELS: u64 = 32_000_000;
const PROTOCOL: &str = "raishot.capture-result-json";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Capabilities {
    protocol: String,
    version: u32,
    command: String,
    in_process: bool,
    max_lifetime_ms: u64,
    max_png_bytes: u64,
    max_pixels: u64,
    image_transport: String,
}
pub fn capabilities(code: i32, stdout: &str) -> Result<(), &'static str> {
    if code != 0 || stdout.len() > 8192 {
        return Err("capture_result_unsupported");
    }
    let v: Capabilities = serde_json::from_str(stdout).map_err(|_| "capture_result_unsupported")?;
    if v.protocol != PROTOCOL
        || v.version != 1
        || v.command != "--capture-result-json"
        || !v.in_process
        || v.max_lifetime_ms != 30_000
        || v.max_png_bytes != MAX_BYTES
        || v.max_pixels != MAX_PIXELS
        || v.image_transport != "owned_png_file"
    {
        return Err("capture_result_unsupported");
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    pub file_name: String,
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub byte_length: String,
    pub sha256: String,
}
impl Image {
    pub fn bytes(&self) -> Option<u64> {
        if self.byte_length.starts_with('0')
            || self.byte_length.len() > 8
            || !self.byte_length.bytes().all(|c| c.is_ascii_digit())
        {
            return None;
        }
        self.byte_length
            .parse()
            .ok()
            .filter(|n| (1..=MAX_BYTES).contains(n))
    }
    fn valid(&self) -> bool {
        self.file_name == "image.png"
            && self.format == "png"
            && self.bytes().is_some()
            && self.width > 0
            && self.height > 0
            && self.width <= 65535
            && self.height <= 65535
            && u64::from(self.width) * u64::from(self.height) <= MAX_PIXELS
            && hex(&self.sha256, 64)
    }
}
pub fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn error_code(value: &str) -> bool {
    (1..=64).contains(&value.len()) && value.bytes().all(|c| c.is_ascii_lowercase() || c == b'_')
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Terminal {
    protocol: String,
    version: u32,
    pub request_id: String,
    completion_scope: String,
    pub status: String,
    pub image: Option<Image>,
    pub error_code: Option<String>,
    pub image_effect_state: String,
    automatic_clipboard_written: bool,
    automatic_history_added: bool,
    source_auto_adsorb_verified: bool,
    retention: String,
    pub warning_code: Option<String>,
}
pub fn terminal(id: &str, code: i32, stdout: &str) -> Result<Terminal, &'static str> {
    let bad = "capture_result_unconfirmed";
    if !hex(id, 32) || stdout.len() > 8192 {
        return Err(bad);
    }
    // serde 的 deny_unknown_fields 与结构字段反序列化同时拒绝多余、重复字段。
    let v: Terminal = serde_json::from_str(stdout).map_err(|_| bad)?;
    // Option 字段必须显式存在；serde 对缺失 Option 的默认行为不能作为协议放宽。
    let raw: serde_json::Value = serde_json::from_str(stdout).map_err(|_| bad)?;
    if raw.as_object().map_or(0, |o| o.len()) != 13
        || v.protocol != PROTOCOL
        || v.version != 1
        || v.request_id != id
        || v.completion_scope != "png_file_only"
        || v.automatic_clipboard_written
        || v.automatic_history_added
        || v.source_auto_adsorb_verified
        || v.retention != "caller_owned_until_explicit_release"
    {
        return Err(bad);
    }
    match v.status.as_str() {
        "completed"
            if code == 0
                && v.error_code.is_none()
                && v.image_effect_state == "confirmed"
                && v.image.as_ref().is_some_and(Image::valid)
                && matches!(
                    v.warning_code.as_deref(),
                    None | Some("worker_cleanup_failed")
                ) =>
        {
            Ok(v)
        }
        "cancelled" | "failed"
            if code == 1
                && v.image.is_none()
                && v.error_code.as_deref().is_some_and(error_code)
                && matches!(v.image_effect_state.as_str(), "not_started" | "unknown")
                && v.warning_code.is_none() =>
        {
            Ok(v)
        }
        _ => Err(bad),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn completed() -> serde_json::Value {
        serde_json::json!({"protocol":PROTOCOL,"version":1,"request_id":"a".repeat(32),
            "completion_scope":"png_file_only","status":"completed","image":{"file_name":"image.png","format":"png",
                "width":1,"height":1,"byte_length":"70","sha256":"b".repeat(64)},
            "error_code":null,"image_effect_state":"confirmed","automatic_clipboard_written":false,
            "automatic_history_added":false,"source_auto_adsorb_verified":false,
            "retention":"caller_owned_until_explicit_release","warning_code":null})
    }
    #[test]
    fn complete_metadata_is_not_yet_file_verification() {
        let value = terminal(&"a".repeat(32), 0, &completed().to_string()).unwrap();
        assert!(value.image.is_some()); // 实际文件、锁、长度、PNG 头和哈希由 Session::verify 核验。
    }
    #[test]
    fn duplicate_and_missing_optional_fields_are_rejected() {
        let value = completed().to_string();
        let duplicate = value.replacen('{', "{\"status\":\"completed\",", 1);
        assert!(terminal(&"a".repeat(32), 0, &duplicate).is_err());
        for key in ["image", "error_code", "warning_code"] {
            let mut value = completed();
            value.as_object_mut().unwrap().remove(key);
            assert!(terminal(&"a".repeat(32), 0, &value.to_string()).is_err());
        }
    }
    #[test]
    fn wrong_id_exit_and_side_effect_claims_are_rejected() {
        assert!(terminal(&"c".repeat(32), 0, &completed().to_string()).is_err());
        assert!(terminal(&"a".repeat(32), 1, &completed().to_string()).is_err());
        for key in [
            "automatic_clipboard_written",
            "automatic_history_added",
            "source_auto_adsorb_verified",
        ] {
            let mut value = completed();
            value[key] = serde_json::json!(true);
            assert!(terminal(&"a".repeat(32), 0, &value.to_string()).is_err());
        }
    }
    #[test]
    fn path_payload_oversize_and_pixel_overflow_are_rejected() {
        for name in ["../image.png", "C:\\outside.png", "image.png:ads"] {
            let mut value = completed();
            value["image"]["file_name"] = serde_json::json!(name);
            assert!(terminal(&"a".repeat(32), 0, &value.to_string()).is_err());
        }
        let mut value = completed();
        value["image"]["width"] = serde_json::json!(65535);
        value["image"]["height"] = serde_json::json!(65535);
        assert!(terminal(&"a".repeat(32), 0, &value.to_string()).is_err());
        let mut value = completed();
        value["image"]["base64"] = serde_json::json!("x".repeat(8192));
        assert!(terminal(&"a".repeat(32), 0, &value.to_string()).is_err());
    }
    #[test]
    fn cancellation_is_not_completed_and_warning_preserves_commit() {
        let mut value = completed();
        value["status"] = serde_json::json!("cancelled");
        value["image"] = serde_json::Value::Null;
        value["error_code"] = serde_json::json!("user_cancelled");
        value["image_effect_state"] = serde_json::json!("not_started");
        assert_eq!(
            terminal(&"a".repeat(32), 1, &value.to_string())
                .unwrap()
                .status,
            "cancelled"
        );
        value["image_effect_state"] = serde_json::json!("confirmed");
        assert!(terminal(&"a".repeat(32), 1, &value.to_string()).is_err());
        let mut value = completed();
        value["warning_code"] = serde_json::json!("worker_cleanup_failed");
        assert_eq!(
            terminal(&"a".repeat(32), 0, &value.to_string())
                .unwrap()
                .status,
            "completed"
        );
    }
    #[test]
    fn old_copy_capability_is_not_a_file_result_capability() {
        assert!(capabilities(0, r#"{"protocol":"raishot.capture-copy-json","version":1,"command":"--capture-copy-json","in_process":true}"#).is_err());
    }
    #[test]
    fn exit_zero_and_queued_are_not_completion() {
        assert!(terminal(&"a".repeat(32), 0, r#"{"queued":true}"#).is_err());
    }
    #[test]
    fn relative_file_and_canonical_decimal_are_required() {
        let mut image = Image {
            file_name: "image.png".into(),
            format: "png".into(),
            width: 1,
            height: 1,
            byte_length: "12".into(),
            sha256: "a".repeat(64),
        };
        assert!(image.valid());
        image.file_name = "../image.png".into();
        assert!(!image.valid());
        image.file_name = "image.png".into();
        image.byte_length = "012".into();
        assert!(!image.valid());
    }
}
