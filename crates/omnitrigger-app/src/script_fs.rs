//! 无 UI 的文件元数据与字符串列表适配；文件系统安全检查留在平台层独立复验。
use omnitrigger_actions::{ActionError, CancellationToken};
use omnitrigger_windows::{
    read_local_directory_with_options, stat_local_path, DirectoryEntryType, DirectoryOrder,
    LocalDirectoryEntry, LocalDirectoryOptions, LocalPathKind, LocalStatError, MAX_DIRECTORY_BYTES,
    MAX_DIRECTORY_ENTRIES,
};
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectoryParams {
    path: String,
    max_entries: usize,
    #[serde(default, deserialize_with = "present_option")]
    order: Option<String>,
    #[serde(default, deserialize_with = "present_option")]
    metadata: Option<bool>,
    #[serde(default, deserialize_with = "present_option")]
    entry_type: Option<String>,
    #[serde(default, deserialize_with = "present_option")]
    search_pattern: Option<String>,
    #[serde(default, deserialize_with = "present_option")]
    match_mode: Option<String>,
}

fn present_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

fn directory_error(code: &str) -> ActionError {
    let (code, message) = match code {
        "invalid_input" => (
            "invalid_input",
            "目录枚举需要普通盘符绝对路径及 1–1024 的 max_entries 整数。",
        ),
        "path_not_local" => ("path_not_local", "仅支持直接映射到本地固定卷的盘符。"),
        "path_unavailable" => (
            "path_unavailable",
            "目录含有链接、云端占位或不允许访问的路径属性。",
        ),
        "path_missing" => ("path_missing", "目标目录或其祖先不存在。"),
        "not_directory" => ("not_directory", "目标不是目录。"),
        "access_denied" => ("access_denied", "系统拒绝枚举目录。"),
        "directory_too_large" => (
            "directory_too_large",
            "目录子项数量或回执总大小超过上限；未返回部分结果。",
        ),
        "directory_changed" => (
            "directory_changed",
            "目录在查询期间发生变化，无法确认完整结果。",
        ),
        "fs_unavailable" => ("fs_unavailable", "当前宿主不提供目录枚举。"),
        "cancelled" => ("cancelled", "目录枚举已取消。"),
        "timeout" => ("timeout", "目录枚举超过截止时限。"),
        _ => ("fs_error", "无法枚举本地目录。"),
    };
    let mut error = ActionError::new(code, message);
    error.method = Some("fs.read_directory".into());
    error.effect_state = Some("not_started".into());
    error
}

pub fn read_directory(
    params: Value,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<Value, ActionError> {
    read_directory_with(params, cancellation, deadline, |path, options, check| {
        read_local_directory_with_options(path, options, check)
    })
}

fn read_directory_with(
    params: Value,
    cancellation: &CancellationToken,
    deadline: Instant,
    query: impl FnOnce(
        &str,
        &LocalDirectoryOptions,
        &dyn Fn() -> Result<(), LocalStatError>,
    ) -> Result<Vec<LocalDirectoryEntry>, LocalStatError>,
) -> Result<Value, ActionError> {
    if !params.is_object() {
        return Err(directory_error("invalid_input"));
    }
    let params: DirectoryParams =
        serde_json::from_value(params).map_err(|_| directory_error("invalid_input"))?;
    if !(1..=MAX_DIRECTORY_ENTRIES).contains(&params.max_entries) {
        return Err(directory_error("invalid_input"));
    }
    validate_path(&params.path).map_err(|error| directory_error(&error.code))?;
    let order = match params.order.as_deref() {
        None | Some("name") => DirectoryOrder::Name,
        Some("native") => DirectoryOrder::Native,
        _ => return Err(directory_error("invalid_input")),
    };
    let entry_type = match params.entry_type.as_deref() {
        None | Some("all") => DirectoryEntryType::All,
        Some("file") => DirectoryEntryType::File,
        Some("directory") => DirectoryEntryType::Directory,
        _ => return Err(directory_error("invalid_input")),
    };
    if !matches!(
        (&params.search_pattern, params.match_mode.as_deref()),
        (Some(_), Some("windows-find")) | (None, None)
    ) {
        return Err(directory_error("invalid_input"));
    }
    let options = LocalDirectoryOptions {
        max_entries: params.max_entries,
        order,
        metadata: params.metadata.unwrap_or(false),
        entry_type,
        search_pattern: params.search_pattern,
    };
    options
        .validate()
        .map_err(|error| directory_error(error.code))?;

    let check_state = || check(cancellation, deadline);
    check_state().map_err(|error| directory_error(error.code))?;
    let result = query(&params.path, &options, &check_state);
    // OS 调用不可强制抢占；成功和失败都必须经过返回后的取消/时限检查。
    check_state().map_err(|error| directory_error(error.code))?;
    let entries = result.map_err(|error| directory_error(error.code))?;
    if entries.len() > params.max_entries {
        return Err(directory_error("directory_too_large"));
    }
    if options.metadata && entries.iter().any(|entry| entry.metadata.is_none()) {
        return Err(directory_error("fs_error"));
    }
    let entries: Vec<Value> = entries
        .into_iter()
        .map(|entry| {
            let mut value=json!({"name":entry.name,"path":entry.path,"kind":entry.kind,"reason":entry.reason});
            if options.metadata {
                if let Some(m)=entry.metadata {
                    value["metadata"]=json!({"ordinal":m.ordinal,"attributes":m.attributes,
                      "creationTimeUtc100ns":m.creation_utc_100ns.to_string(),"lastAccessTimeUtc100ns":m.access_utc_100ns.to_string(),"lastWriteTimeUtc100ns":m.write_utc_100ns.to_string(),
                      "creationTimeLocalTicks100ns":m.creation_local_ticks.to_string(),"lastAccessTimeLocalTicks100ns":m.access_local_ticks.to_string(),"lastWriteTimeLocalTicks100ns":m.write_local_ticks.to_string(),"timeZoneKey":m.time_zone_key});
                }
            }
            value
        })
        .collect();
    let result = json!({"entries":entries,"complete":true});
    if serde_json::to_vec(&result)
        .map_err(|_| directory_error("fs_error"))?
        .len()
        > MAX_DIRECTORY_BYTES
    {
        return Err(directory_error("directory_too_large"));
    }
    check_state().map_err(|error| directory_error(error.code))?;
    Ok(result)
}

#[cfg(test)]
#[path = "../../../tests/script_fs_read_directory.rs"]
mod directory_tests;

const LOGICAL_SORT_MAX_PATHS: usize = 1024;
const LOGICAL_SORT_MAX_UTF16: usize = 32767;
const LOGICAL_SORT_MAX_BYTES: usize = 64 * 1024;

#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct LogicalSortParams {
    paths: Vec<String>,
}

// 只计数到上限，不为超大直接调用参数构造完整序列化副本。
struct LogicalSortBudget(usize);
impl std::io::Write for LogicalSortBudget {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > LOGICAL_SORT_MAX_BYTES - self.0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "limit",
            ));
        }
        self.0 += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn logical_sort_error(code: &str) -> ActionError {
    let (code, message) = match code {
        "invalid_input" => ("invalid_input", "字符串列表排序参数无效或超过大小限制。"),
        "cancelled" => ("cancelled", "动作已取消，未返回字符串列表排序结果。"),
        "timeout" => ("timeout", "字符串列表排序超过动作运行时限。"),
        _ => ("fs_unavailable", "本机字符串列表排序能力不可用。"),
    };
    let mut error = ActionError::new(code, message);
    error.method = Some("fs.sort_logical_paths".into());
    error.effect_state = Some("not_started".into());
    error
}

pub fn sort_logical_paths(
    params: Value,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<Value, ActionError> {
    sort_logical_paths_with(params, cancellation, deadline, Instant::now, |paths| {
        #[cfg(windows)]
        {
            Ok(omnitrigger_windows::sort_logical_paths(paths))
        }
        #[cfg(not(windows))]
        {
            let _ = paths;
            Err(logical_sort_error("fs_unavailable"))
        }
    })
}

fn sort_logical_paths_with(
    params: Value,
    cancellation: &CancellationToken,
    deadline: Instant,
    now: impl Fn() -> Instant,
    sort: impl FnOnce(&[String]) -> Result<Vec<String>, ActionError>,
) -> Result<Value, ActionError> {
    let check_state = || {
        if cancellation.is_cancelled() {
            Err(logical_sort_error("cancelled"))
        } else if now() >= deadline {
            Err(logical_sort_error("timeout"))
        } else {
            Ok(())
        }
    };
    check_state()?;
    if !params.is_object() {
        return Err(logical_sort_error("invalid_input"));
    }
    let params: LogicalSortParams =
        serde_json::from_value(params).map_err(|_| logical_sort_error("invalid_input"))?;
    if params.paths.len() > LOGICAL_SORT_MAX_PATHS
        || serde_json::to_writer(LogicalSortBudget(0), &params).is_err()
        || params.paths.iter().any(|path| {
            path.encode_utf16().take(LOGICAL_SORT_MAX_UTF16 + 1).count() > LOGICAL_SORT_MAX_UTF16
        })
    {
        return Err(logical_sort_error("invalid_input"));
    }
    check_state()?;
    let result = sort(&params.paths);
    // 系统比较器不可抢占，但本API有界；取消或迟到结果不发布为成功。
    check_state()?;
    let paths = result.map_err(|error| logical_sort_error(&error.code))?;
    Ok(json!(paths))
}

#[cfg(test)]
#[path = "../../../tests/script_fs_logical_sort.rs"]
mod logical_sort_tests;
