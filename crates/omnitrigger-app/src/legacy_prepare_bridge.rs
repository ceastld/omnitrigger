//! 既有plugin_install worker的内部准备适配；不捕获窗口目标或提交业务。
//! 复用同一ScriptHostRuntime/原process.capture适配，不创建ActionService/线程/进程管理器。
use crate::legacy_load_gate::PrepareResult;
use omnitrigger_actions::{
    ActionError, ActionRuntime, CancellationToken, InvocationContext, NativeHost, ResolvedAction,
    ScriptHostRuntime,
};
use serde_json::{json, Value};

pub(crate) struct NoTargetHost;
impl NativeHost for NoTargetHost {
    fn call(&self, method: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
        let mut error = ActionError::new(
            "preparation_api_forbidden",
            "加载预备没有窗口、输入或剪贴板上下文。",
        );
        error.effect_state = Some("not_started".into());
        error.method = Some(method.into());
        Err(error)
    }
}

/// action只能由已验证安装收据构造，entry/helper/固定scope模块须校验哈希；不从公共调用输入构造。
/// caller沿用script_runtime::Installed.runtime()的完整性检查和60秒预算。
pub(crate) fn prepare_verified_host(
    runtime: &ScriptHostRuntime,
    action: &ResolvedAction,
    context: &InvocationContext,
    cancellation: &CancellationToken,
) -> Result<PrepareResult, ActionError> {
    if action.definition.runtime != "script"
        || action.definition.entry != "prepare-host.mjs"
        || action.entry_path != action.package_root.join("prepare-host.mjs")
        || action.call.input != json!({})
        || context.binding_id != 0
    {
        let mut error =
            ActionError::new("invalid_preparation_entry", "加载预备入口与已验证包不符。");
        error.effect_state = Some("not_started".into());
        return Err(error);
    }
    let host = crate::process_capture_host::WithProcessCapture::new(NoTargetHost);
    // 不经过action_packages::submit；它会先DesktopHost::capture。
    // 原SDK的process.start/delay原语保持原实现；NativeHost只接收process.capture。
    let output = runtime.execute_with_host(action, context, cancellation, Some(&host))?;
    if output.as_object().map(|value| value.len()) != Some(6)
        || output.get("schema_version").and_then(Value::as_u64) != Some(1)
        || output.get("scope").and_then(Value::as_str) != Some("legacy_load_preparation")
        || output.get("businessDispatched").and_then(Value::as_u64) != Some(0)
        || output
            .get("serverStartRequested")
            .and_then(Value::as_bool)
            .is_none()
        || !matches!(
            output.get("state").and_then(Value::as_str),
            Some("ready" | "not_ready")
        )
        || output
            .get("errorCode")
            .and_then(Value::as_str)
            .is_none_or(|code| {
                code.len() > 64
                    || !code
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            })
    {
        let mut error = ActionError::new("invalid_preparation_reply", "加载预备未返回有效状态。");
        error.effect_state = Some("unknown".into()); // 宿主可能已启动；这不是业务效果。
        return Err(error);
    }
    if output.get("state").and_then(Value::as_str) == Some("ready")
        && output.get("errorCode").and_then(Value::as_str) == Some("")
    {
        return Ok(PrepareResult::Ready);
    }
    if output.get("state").and_then(Value::as_str) != Some("not_ready")
        || output.get("errorCode").and_then(Value::as_str) == Some("")
    {
        let mut error = ActionError::new(
            "invalid_preparation_reply",
            "加载预备状态与错误代码不一致。",
        );
        error.effect_state = Some("unknown".into());
        return Err(error);
    }
    let code = match output.get("errorCode").and_then(Value::as_str) {
        Some("host_stopping") => "host_stopping",
        Some("host_preparation_timeout") => "host_preparation_timeout",
        Some("host_not_ready") => "host_not_ready",
        Some("access_denied") => "access_denied",
        Some("cancelled") => "cancelled",
        Some("server_start_unconfirmed") => "server_start_unconfirmed",
        _ => "legacy_preparation_failed",
    };
    Ok(PrepareResult::NotReady(code))
}
