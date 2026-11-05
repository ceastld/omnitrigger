//! 无 UI 的只读文件元数据适配；本机安全检查留在 Windows 平台层独立复验。
use omnitrigger_actions::{ActionError, CancellationToken};
use omnitrigger_windows::{stat_local_path, LocalPathKind, LocalStatError};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Instant;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Params {
    path: String,
}

fn fault(code: &str, message: &str) -> ActionError {
    let mut error = ActionError::new(code, message);
    error.method = Some("fs.stat".into());
    error.effect_state = Some("not_started".into());
    error
}

fn invalid() -> ActionError {
    fault(
        "invalid_input",
        "文件元数据查询需要普通盘符绝对路径，且只接受 path 字符串参数。",
    )
}

fn reserved(component: &str) -> bool {
    // Windows 设备名在扩展名前仍保留；名称前不做 trim 或路径规范化。
    let stem = component
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    stem.strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"))
        .is_some_and(|suffix| {
            let mut chars = suffix.chars();
            matches!(chars.next(), Some('1'..='9' | '¹' | '²' | '³')) && chars.next().is_none()
        })
}

fn validate_path(path: &str) -> Result<(), ActionError> {
    let bytes = path.as_bytes();
    if !(1..=32767).contains(&path.encode_utf16().count())
        || path.chars().any(char::is_control)
        || bytes.len() < 3
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1] != b':'
        || !matches!(bytes[2], b'\\' | b'/')
    {
        return Err(invalid());
    }
    let tail = &path[3..]; // 前三字节已核验为 ASCII 盘符、冒号和分隔符。
    if tail.is_empty() {
        return Ok(());
    }
    let tail = tail.strip_suffix(['\\', '/']).unwrap_or(tail);
    for component in tail.split(['\\', '/']) {
        if component.is_empty()
            || matches!(component, "." | "..")
            || component.ends_with([' ', '.'])
            || component
                .chars()
                .any(|ch| matches!(ch, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
            || reserved(component)
        {
            return Err(invalid());
        }
    }
    if tail.split(['\\', '/']).any(|component| {
        let lower = component.to_ascii_lowercase();
        lower.starts_with("onedrive") || lower.starts_with("icloud") || lower == "dropbox"
    }) {
        return Err(fault("path_unavailable", "不查询云盘同步目录中的路径。"));
    }
    Ok(())
}

fn check(cancellation: &CancellationToken, deadline: Instant) -> Result<(), LocalStatError> {
    if cancellation.is_cancelled() {
        return Err(LocalStatError {
            code: "cancelled",
            message: "动作已取消，未继续查询文件元数据。".into(),
        });
    }
    if Instant::now() >= deadline {
        return Err(LocalStatError {
            code: "timeout",
            message: "文件元数据查询超过动作运行时限。".into(),
        });
    }
    Ok(())
}

fn platform_error(error: LocalStatError) -> ActionError {
    // 不直接拼接 OS 或平台错误正文，避免未来适配器意外回显路径、凭据等信息。
    let (code, message) = match error.code {
        "invalid_input" => ("invalid_input", "文件路径不符合本机元数据查询要求。"),
        "path_not_local" => ("path_not_local", "仅支持安全可查询的本地固定磁盘路径。"),
        "path_unavailable" => (
            "path_unavailable",
            "路径存在链接、联接点、云端占位属性或其他不可安全查询状态。",
        ),
        "access_denied" => ("access_denied", "没有查询文件元数据的访问权限。"),
        "fs_unavailable" => ("fs_unavailable", "本机文件元数据能力不可用。"),
        "cancelled" => ("cancelled", "动作已取消，未继续查询文件元数据。"),
        "timeout" => ("timeout", "文件元数据查询超过动作运行时限。"),
        _ => ("fs_error", "无法查询本机文件元数据。"),
    };
    fault(code, message)
}

pub fn stat(
    params: Value,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<Value, ActionError> {
    stat_with(params, cancellation, deadline, |path, check| {
        stat_local_path(path, check)
    })
}

fn stat_with(
    params: Value,
    cancellation: &CancellationToken,
    deadline: Instant,
    query: impl FnOnce(
        &str,
        &dyn Fn() -> Result<(), LocalStatError>,
    ) -> Result<LocalPathKind, LocalStatError>,
) -> Result<Value, ActionError> {
    let params: Params = serde_json::from_value(params).map_err(|_| invalid())?;
    validate_path(&params.path)?;
    let check_state = || check(cancellation, deadline);
    check_state().map_err(platform_error)?;
    let result = query(&params.path, &check_state);
    // 单次 OS 调用不可强制抢占；返回后仍检查取消/时限，不将迟到结果当成功。
    check_state().map_err(platform_error)?;
    let kind = match result.map_err(platform_error)? {
        LocalPathKind::File => "file",
        LocalPathKind::Directory => "directory",
        LocalPathKind::Missing => "missing",
        LocalPathKind::Other => "other",
    };
    Ok(json!({"kind":kind}))
}

#[cfg(test)]
#[path = "../../../tests/script_fs_stat.rs"]
mod tests;
