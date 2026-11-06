//! 既有plugin_install worker的Legacy作业叶；不持有线程/调度器/目标/业务队列。
use crate::legacy_load_gate::{LegacyKey, PrepareResult, PrepareTicket};
use omnitrigger_actions::{
    ActionError, CancellationToken, InvocationContext, NativeHost, ResolvedAction,
};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

pub(crate) const PACKAGE_ID: &str = "omnitrigger.legacy";

/// 复用安装产物完整性的接缝，不读取用户ActionCall字段，不引入profile审批。
/// key()只返回现有安装索引标识；verify()在worker沿既有机制核验文件和本地路径。
pub(crate) trait LegacyArtifactSource: Send + Sync {
    fn key(&self) -> LegacyKey;
    fn verify(
        &self,
        config: &Path,
        cancellation: &CancellationToken,
    ) -> Result<VerifiedLegacyInstall, ActionError>;
}

#[derive(Clone, Debug)]
pub(crate) struct VerifiedLegacyInstall {
    pub key: LegacyKey,
    pub executable: PathBuf,
    pub prepare_action: ResolvedAction,
}
impl VerifiedLegacyInstall {
    fn validate(&self, expected: &LegacyKey) -> Result<(), ActionError> {
        let action = &self.prepare_action;
        let hash =
            |value: &str| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit());
        if &self.key != expected
            || self.key.package_id != PACKAGE_ID
            || !hash(&self.key.artifact_fingerprint)
            || self.key.fixed_scope.is_empty()
            || !self.executable.is_absolute()
            || !action.package_root.is_absolute()
            || self.executable
                != action
                    .package_root
                    .join("host")
                    .join("OmniTrigger.LegacyHost.exe")
            || action.call.package_id != PACKAGE_ID
            || action.call.action_id != "__prepare"
            || action.call.action_version != 1
            || action.call.input != json!({})
            || action.definition.id != "__prepare"
            || action.definition.version != 1
            || action.definition.runtime != "script"
            || action.definition.entry != "prepare-host.mjs"
            || action.entry_path != action.package_root.join("prepare-host.mjs")
        {
            return Err(fault(
                "legacy_receipt_mismatch",
                "正式宿主安装收据与内部准备入口不一致。",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LegacyOperation {
    Prepare,
    Stop,
}

pub(crate) struct LegacyJob {
    pub ticket: PrepareTicket,
    pub operation: LegacyOperation,
    pub config: PathBuf,
    pub source: Arc<dyn LegacyArtifactSource>,
    pub cancellation: CancellationToken,
}
#[derive(Debug)]
pub(crate) struct LegacyFinished {
    pub ticket: PrepareTicket,
    pub operation: LegacyOperation,
    pub installation: Option<VerifiedLegacyInstall>,
    pub result: Result<LegacyCompletion, ActionError>,
}
#[derive(Debug)]
pub(crate) enum LegacyCompletion {
    Prepared(PrepareResult),
    EndpointAlreadyAbsent,
    ShutdownAccepted,
    EndpointStopping,
}

pub(crate) fn run(job: LegacyJob) -> LegacyFinished {
    let mut installation = None;
    let result = (|| {
        if job.cancellation.is_cancelled() {
            return Err(fault("cancelled", "宿主准备已取消。"));
        }
        let verified = job.source.verify(&job.config, &job.cancellation)?;
        verified.validate(&job.ticket.key)?;
        installation = Some(verified.clone()); // 准备出错可能已经serve；保留已核验收据供显式停用。
        if job.cancellation.is_cancelled() {
            return Err(fault("cancelled", "宿主准备已取消。"));
        }
        match job.operation {
            LegacyOperation::Prepare => {
                let runtime = crate::script_runtime::locate(&job.config)
                    .map_err(|message| fault("legacy_script_runtime_unavailable", &message))?
                    .ok_or_else(|| {
                        fault(
                            "legacy_script_runtime_unavailable",
                            "轻量脚本宿主尚未安装。",
                        )
                    })?
                    .runtime();
                crate::legacy_prepare_bridge::prepare_verified_host(
                    &runtime,
                    &verified.prepare_action,
                    &InvocationContext {
                        invocation_id: job.ticket.generation,
                        binding_id: 0,
                    },
                    &job.cancellation,
                )
                .map(LegacyCompletion::Prepared)
            }
            LegacyOperation::Stop => stop_verified(&verified.executable, &job.cancellation),
        }
    })();
    LegacyFinished {
        ticket: job.ticket,
        operation: job.operation,
        installation,
        result,
    }
}

fn fault(code: &str, message: &str) -> ActionError {
    let mut error = ActionError::new(code, message);
    error.effect_state = Some("not_started".into());
    error
}

fn capture(
    host: &impl NativeHost,
    executable: &Path,
    mode: &str,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<Value, ActionError> {
    let program = executable
        .to_str()
        .ok_or_else(|| fault("legacy_invalid_path", "宿主路径不能表示为有效字符串。"))?;
    let captured = host.call_with_deadline(
        "process.capture",
        json!({"program":program,"args":[mode],
        "timeout_ms":5000,"allow_non_zero":true}),
        cancellation,
        deadline,
    )?;
    if captured.get("code").and_then(Value::as_i64) != Some(0)
        || captured.get("stderr").and_then(Value::as_str) != Some("")
    {
        return Err(fault(
            "legacy_control_unavailable",
            "未获得有效宿主控制回执，未重试。",
        ));
    }
    serde_json::from_str(captured.get("stdout").and_then(Value::as_str).unwrap_or(""))
        .map_err(|_| fault("legacy_control_invalid_reply", "宿主控制回执无效，未重试。"))
}

fn stop_verified(
    executable: &Path,
    cancellation: &CancellationToken,
) -> Result<LegacyCompletion, ActionError> {
    let host = crate::process_capture_host::WithProcessCapture::new(
        crate::legacy_prepare_bridge::NoTargetHost,
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    // 只观察一次状态，再至多一次正常stop；无循环、保活、强杀或PID名字匹配。
    let status = capture(&host, executable, "status", cancellation, deadline)?;
    if status.as_object().map(|value| value.len()) != Some(5)
        || status.get("schema_version").and_then(Value::as_u64) != Some(1)
        || status.get("scope").and_then(Value::as_str) != Some("host_status")
    {
        return Err(fault("legacy_control_invalid_reply", "宿主状态回执无效。"));
    }
    match (
        status.get("state").and_then(Value::as_str),
        status.get("can_start").and_then(Value::as_bool),
        status.get("error_code"),
    ) {
        (Some("absent"), Some(true), Some(Value::String(code))) if code == "endpoint_missing" => {
            return Ok(LegacyCompletion::EndpointAlreadyAbsent)
        }
        (Some("stopping"), Some(false), Some(Value::Null)) => {
            return Ok(LegacyCompletion::EndpointStopping)
        }
        (Some("ready" | "starting" | "failed"), Some(false), Some(Value::Null)) => {}
        _ => {
            return Err(fault(
                "legacy_stop_unconfirmed",
                "未确认可正常停止的宿主状态；插件保持停用。",
            ))
        }
    }
    let reply =
        capture(&host, executable, "stop", cancellation, deadline).map_err(|mut error| {
            error.effect_state = Some("unknown".into());
            error
        })?;
    decode_stop(&reply)
}

fn decode_stop(reply: &Value) -> Result<LegacyCompletion, ActionError> {
    let id_valid = reply
        .get("request_id")
        .and_then(Value::as_str)
        .is_some_and(|value| {
            value.len() == 36
                && value.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_hexdigit()
                    }
                })
        });
    if reply.as_object().map(|value| value.len()) == Some(5)
        && id_valid
        && reply.get("schema_version").and_then(Value::as_u64) == Some(1)
        && reply.get("scope").and_then(Value::as_str) == Some("host_shutdown_request")
        && reply.get("outcome").and_then(Value::as_str) == Some("Succeeded")
        && reply.get("error_code") == Some(&Value::Null)
    {
        return Ok(LegacyCompletion::ShutdownAccepted);
    }
    let mut error = fault(
        "legacy_stop_unconfirmed",
        "宿主停机未确认；插件保持停用，不自动重试。",
    );
    error.effect_state = Some("unknown".into());
    Err(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn reply() -> Value {
        json!({"schema_version":1,"scope":"host_shutdown_request","request_id":"11111111-2222-3333-4444-555555555555","outcome":"Succeeded","error_code":null})
    }
    #[test]
    fn success_is_only_shutdown_acceptance_not_process_exit() {
        assert!(matches!(
            decode_stop(&reply()),
            Ok(LegacyCompletion::ShutdownAccepted)
        ));
    }
    #[test]
    fn wrong_scope_extra_field_and_unknown_never_confirm_stop() {
        for key in ["scope", "extra", "outcome"] {
            let mut value = reply();
            value[key] = json!("Unknown");
            assert_eq!(
                decode_stop(&value).unwrap_err().effect_state.as_deref(),
                Some("unknown")
            );
        }
    }
}
