//! 外部桌面宿主提交动作时的短时目标快照；不捕获当前前台。
use omnitrigger_windows::ScriptTargetSnapshot;
use serde::{Deserialize, Serialize};
use std::{io::Read, path::Path, time::SystemTime};

pub fn validate_cli(arguments: &[String]) -> Result<(), String> {
    let mut index = usize::from(arguments.first().is_some_and(|arg| !arg.starts_with("--")));
    let mut seen = std::collections::BTreeSet::new();
    while index < arguments.len() {
        let flag = arguments[index].as_str();
        if !matches!(
            flag,
            "--config"
                | "--check-action"
                | "--run-action"
                | "--target-context"
                | "--list-package-actions"
                | "--create-script-action"
                | "--install-script-runtime"
                | "--sha256"
                | "--stream-action-events"
        ) || !seen.insert(flag)
        {
            return Err(format!("动作命令含未知或重复参数：{flag}"));
        }
        if matches!(flag, "--list-package-actions" | "--stream-action-events") {
            index += 1;
            continue;
        }
        if arguments
            .get(index + 1)
            .is_none_or(|value| value.starts_with("--"))
        {
            return Err(if flag == "--create-script-action" {
                "--create-script-action 后需要工程标识。".into()
            } else {
                format!("参数 {flag} 后需要文件路径。")
            });
        }
        index += 2;
    }
    let commands = [
        "--check-action",
        "--run-action",
        "--list-package-actions",
        "--create-script-action",
        "--install-script-runtime",
    ]
    .iter()
    .filter(|flag| seen.contains(**flag))
    .count();
    if commands != 1 {
        return Err(
            "需要且只能指定一次 --check-action、--run-action、--list-package-actions、--create-script-action 或 --install-script-runtime。".into(),
        );
    }
    if seen.contains("--install-script-runtime") != seen.contains("--sha256") {
        return Err("--install-script-runtime 需要且只能搭配 --sha256 使用。".into());
    }
    if (seen.contains("--target-context") || seen.contains("--stream-action-events"))
        && !seen.contains("--run-action")
    {
        return Err("目标上下文和事件流只能用于 --run-action。".into());
    }
    Ok(())
}

pub fn path(arguments: &[String]) -> Result<Option<&Path>, String> {
    let indexes: Vec<_> = arguments
        .iter()
        .enumerate()
        .filter_map(|(index, arg)| (arg == "--target-context").then_some(index))
        .collect();
    if indexes.is_empty() {
        return Ok(None);
    }
    if indexes.len() != 1
        || arguments
            .iter()
            .filter(|arg| *arg == "--run-action")
            .count()
            != 1
        || arguments.iter().any(|arg| arg == "--check-action")
    {
        return Err(
            "--target-context 需要且只能搭配一次 --run-action，不能重复或用于只读检查。".into(),
        );
    }
    arguments
        .get(indexes[0] + 1)
        .filter(|value| !value.starts_with("--"))
        .map(|value| Some(Path::new(value)))
        .ok_or_else(|| "--target-context 后需要目标快照文件路径。".into())
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TargetContext {
    window_handle: String,
    process_id: u32,
    thread_id: u32,
    process_created_file_time: String,
    expires_at_unix_ms: u64,
}

pub fn export(snapshot: ScriptTargetSnapshot) -> Result<serde_json::Value, String> {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_millis();
    let expires_at_unix_ms = u64::try_from(now)
        .map_err(|_| "系统时间超出范围。")?
        .checked_add(30_000)
        .ok_or("系统时间超出范围。")?;
    serde_json::to_value(TargetContext {
        window_handle: snapshot.window_handle.to_string(),
        process_id: snapshot.process_id,
        thread_id: snapshot.thread_id,
        process_created_file_time: snapshot.process_created_file_time.to_string(),
        expires_at_unix_ms,
    })
    .map_err(|error| error.to_string())
}

fn decimal<T: std::str::FromStr>(value: &str) -> Result<T, String> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("句柄和进程创建时间必须为十进制字符串。".into());
    }
    value.parse().map_err(|_| "目标快照数值超出范围。".into())
}

impl TargetContext {
    fn validate(&self, now: u64) -> Result<ScriptTargetSnapshot, String> {
        if self.expires_at_unix_ms <= now || self.expires_at_unix_ms > now.saturating_add(30_000) {
            return Err("目标快照已过期或有效期超过 30 秒。".into());
        }
        let snapshot = ScriptTargetSnapshot {
            window_handle: decimal(&self.window_handle)?,
            process_id: self.process_id,
            thread_id: self.thread_id,
            process_created_file_time: decimal(&self.process_created_file_time)?,
        };
        if snapshot.window_handle == 0
            || snapshot.window_handle > isize::MAX as usize
            || snapshot.process_id == 0
            || snapshot.thread_id == 0
            || snapshot.process_created_file_time == 0
        {
            return Err("目标快照身份不完整。".into());
        }
        Ok(snapshot)
    }
}

/// 应用内目标交接复用相同DTO与有效期规则，不读取快照文件。
pub fn validate_value(value: serde_json::Value, now: u64) -> Result<ScriptTargetSnapshot, String> {
    let context: TargetContext =
        serde_json::from_value(value).map_err(|_| "目标快照格式无效。".to_string())?;
    context.validate(now)
}

pub fn read(path: &Path) -> Result<ScriptTargetSnapshot, String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|error| format!("读取目标快照失败：{error}"))?
        .take(4097)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > 4096 {
        return Err("目标快照不能超过 4 KiB。".into());
    }
    let context: TargetContext =
        serde_json::from_slice(&bytes).map_err(|error| format!("目标快照格式有误：{error}"))?;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_millis()
        .try_into()
        .map_err(|_| "系统时间超出范围。")?;
    context.validate(now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn value() -> serde_json::Value {
        json!({"window_handle":"123", "process_id":7, "thread_id":8,
            "process_created_file_time":"134386985330123456", "expires_at_unix_ms":1100})
    }

    #[test]
    fn target_context_option_rejects_ambiguous_or_missing_action_arguments() {
        for args in [
            vec!["--target-context", "target.json"],
            vec!["--run-action", "a.json", "--target-context"],
            vec![
                "--run-action",
                "a.json",
                "--target-context",
                "--config",
                "c.json",
            ],
            vec![
                "--run-action",
                "a.json",
                "--target-context",
                "t.json",
                "--target-context",
                "u.json",
            ],
            vec!["--check-action", "a.json", "--target-context", "t.json"],
            vec![
                "--run-action",
                "a.json",
                "--run-action",
                "b.json",
                "--target-context",
                "t.json",
            ],
        ] {
            assert!(path(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>()).is_err());
        }
        assert!(path(&["--check-action".into(), "a.json".into()])
            .unwrap()
            .is_none());
        assert_eq!(
            path(&[
                "--run-action".into(),
                "a.json".into(),
                "--target-context".into(),
                "t.json".into()
            ])
            .unwrap(),
            Some(Path::new("t.json"))
        );
    }

    #[test]
    fn misspelled_target_option_cannot_fall_back_to_current_foreground() {
        for args in [
            vec![
                "app.exe",
                "--run-action",
                "a.json",
                "--target-contex",
                "t.json",
            ],
            vec!["--run-action", "a.json", "extra.json"],
            vec![
                "--run-action",
                "a.json",
                "--config",
                "a.json",
                "--config",
                "b.json",
            ],
            vec!["--run-action", "a.json", "--check-action", "a.json"],
        ] {
            assert!(validate_cli(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>()).is_err());
        }
        assert!(validate_cli(&[
            "app.exe".into(),
            "--run-action".into(),
            "a.json".into(),
            "--target-context".into(),
            "t.json".into()
        ])
        .is_ok());
    }

    #[test]
    fn package_catalog_and_event_stream_have_unambiguous_cli_modes() {
        for args in [
            vec!["app.exe", "--list-package-actions"],
            vec!["app.exe", "--create-script-action", "my-script"],
            vec!["--install-script-runtime", "host.zip", "--sha256", "digest"],
            vec!["--config", "owned.json", "--list-package-actions"],
            vec![
                "--run-action",
                "call.json",
                "--stream-action-events",
                "--target-context",
                "target.json",
            ],
        ] {
            assert!(validate_cli(&args.iter().map(|x| x.to_string()).collect::<Vec<_>>()).is_ok());
        }
        for args in [
            vec!["--stream-action-events"],
            vec!["--list-package-actions", "extra"],
            vec!["--create-script-action"],
            vec!["--install-script-runtime", "host.zip"],
            vec!["--create-script-action", "script", "--sha256", "digest"],
            vec![
                "--install-script-runtime",
                "host.zip",
                "--sha256",
                "digest",
                "--run-action",
                "call.json",
            ],
            vec![
                "--create-script-action",
                "my-script",
                "--run-action",
                "a.json",
            ],
            vec![
                "--create-script-action",
                "my-script",
                "--stream-action-events",
            ],
            vec![
                "--create-script-action",
                "my-script",
                "--target-context",
                "target.json",
            ],
            vec!["--list-package-actions", "--list-package-actions"],
            vec!["--list-package-actions", "--run-action", "call.json"],
            vec!["--list-package-actions", "--target-context", "target.json"],
            vec!["--check-action", "call.json", "--stream-action-events"],
            vec![
                "--run-action",
                "call.json",
                "--stream-action-events",
                "--stream-action-events",
            ],
        ] {
            assert!(validate_cli(&args.iter().map(|x| x.to_string()).collect::<Vec<_>>()).is_err());
        }
    }

    #[test]
    fn target_context_requires_full_identity_and_preserves_integer_precision() {
        let mut value = value();
        let context: TargetContext = serde_json::from_value(value.clone()).unwrap();
        let snapshot = context.validate(1000).unwrap();
        assert_eq!(snapshot.process_created_file_time, 134386985330123456);
        for field in [
            "window_handle",
            "process_id",
            "thread_id",
            "process_created_file_time",
        ] {
            let mut missing = value.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<TargetContext>(missing).is_err());
        }
        value["window_handle"] = json!(123);
        assert!(serde_json::from_value::<TargetContext>(value).is_err());
    }

    #[test]
    fn target_context_rejects_expiry_invalid_numbers_and_unknown_fields() {
        let context: TargetContext = serde_json::from_value(value()).unwrap();
        assert!(context.validate(1100).is_err());
        assert!(context.validate(0).is_ok());
        let mut future = value();
        future["expires_at_unix_ms"] = json!(31_001);
        assert!(serde_json::from_value::<TargetContext>(future)
            .unwrap()
            .validate(1000)
            .is_err());
        for (field, bad) in [
            ("window_handle", json!("-1")),
            ("window_handle", json!("0")),
            ("process_created_file_time", json!("18446744073709551616")),
            ("process_created_file_time", json!("+123")),
            ("process_id", json!(0)),
            ("thread_id", json!(0)),
        ] {
            let mut invalid = value();
            invalid[field] = bad;
            assert!(serde_json::from_value::<TargetContext>(invalid)
                .unwrap()
                .validate(1000)
                .is_err());
        }
        let mut extra = value();
        extra["fallback_to_foreground"] = json!(true);
        assert!(serde_json::from_value::<TargetContext>(extra).is_err());
    }
}
