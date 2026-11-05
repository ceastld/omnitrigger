//! 仅显式要求能力时运行宿主自述；不传动作输入，不读取或执行动作入口。
use super::{error, OwnedHost};
use crate::runtime_capabilities::{
    validate_requirements, RuntimeDescription, MAX_DESCRIPTION_BYTES,
};
use crate::{ActionError, CancellationToken};
use std::{
    fs::File,
    io::{Read, Seek},
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub(super) fn check(
    program: &Path,
    required: &[String],
    timeout: Duration,
    cancellation: &CancellationToken,
) -> Result<(), ActionError> {
    let convert = |fault: crate::runtime_capabilities::CapabilityError| {
        let message = if fault.code == "runtime_capabilities_unknown" {
            format!(
                "{}所需能力：{}。",
                fault.message,
                fault.missing_capabilities.join("、")
            )
        } else {
            fault.message
        };
        error(&fault.code, message, "not_started")
    };
    validate_requirements(required).map_err(convert)?;
    if required.is_empty() {
        return Ok(());
    }
    if cancellation.is_cancelled() {
        return Err(error(
            "cancelled",
            "能力探测前已取消，未运行动作。",
            "not_started",
        ));
    }
    if timeout.is_zero() {
        return Err(error(
            "runtime_description_timeout",
            "宿主能力探测超时，未运行动作。",
            "not_started",
        ));
    }
    let scratch = crate::node::ResponseDirectory::new().map_err(|_| {
        error(
            "runtime_description_io",
            "无法创建能力探测目录，未运行上层动作。",
            "not_started",
        )
    })?;
    let mut output = File::options()
        .create_new(true)
        .read(true)
        .write(true)
        .open(scratch.response())
        .map_err(|_| {
            error(
                "runtime_description_io",
                "无法创建宿主能力探测回执，未运行动作。",
                "not_started",
            )
        })?;
    let mut command = Command::new(program);
    command
        .arg("--describe-runtime")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(output.try_clone().map_err(|_| {
            error(
                "runtime_description_io",
                "无法创建宿主能力探测回执，未运行动作。",
                "not_started",
            )
        })?);
    crate::node::hide_window(&mut command);
    let mut child = OwnedHost(command.spawn().map_err(|_| {
        error(
            "runtime_description_start",
            "无法探测宿主能力，未运行动作。",
            "not_started",
        )
    })?);
    let began = Instant::now();
    loop {
        if cancellation.is_cancelled() {
            return Err(error(
                "cancelled",
                "能力探测已取消，未运行动作。",
                "not_started",
            ));
        }
        if began.elapsed() >= timeout.min(Duration::from_secs(10)) {
            return Err(error(
                "runtime_description_timeout",
                "宿主能力探测超时，未运行动作。",
                "not_started",
            ));
        }
        if output
            .metadata()
            .map_err(|_| {
                error(
                    "runtime_description_io",
                    "无法读取宿主能力回执，未运行动作。",
                    "not_started",
                )
            })?
            .len()
            > MAX_DESCRIPTION_BYTES as u64
        {
            return Err(error(
                "runtime_description_invalid",
                "宿主能力回执超过 4096 字节，未运行动作。",
                "not_started",
            ));
        }
        if let Some(status) = child.0.try_wait().map_err(|_| {
            error(
                "runtime_description_io",
                "无法等待宿主能力回执，未运行动作。",
                "not_started",
            )
        })? {
            if !status.success() {
                return Err(error(
                    "runtime_description_failed",
                    "宿主能力探测失败，未运行动作。",
                    "not_started",
                ));
            }
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    output.rewind().map_err(|_| {
        error(
            "runtime_description_io",
            "无法读取宿主能力回执，未运行动作。",
            "not_started",
        )
    })?;
    let mut bytes = Vec::new();
    output
        .take(MAX_DESCRIPTION_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            error(
                "runtime_description_io",
                "无法读取宿主能力回执，未运行动作。",
                "not_started",
            )
        })?;
    RuntimeDescription::parse(&bytes)
        .map_err(convert)?
        .check(required)
        .map_err(convert)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_requirements_does_not_probe_missing_program() {
        check(
            Path::new("missing-synthetic-host"),
            &[],
            Duration::ZERO,
            &CancellationToken::new(),
        )
        .unwrap();
    }
    #[test]
    fn invalid_requirements_are_rejected_before_probe() {
        let fault = check(
            Path::new("missing-synthetic-host"),
            &["bad/token".into()],
            Duration::from_secs(1),
            &CancellationToken::new(),
        )
        .unwrap_err();
        assert_eq!(fault.code, "invalid_runtime_requirements");
        assert_eq!(fault.effect_state.as_deref(), Some("not_started"));
    }
    #[test]
    fn cancelled_or_zero_budget_does_not_start_probe() {
        let token = CancellationToken::new();
        token.cancel();
        let required = vec!["clipboard.history.v1".into()];
        let fault = check(
            Path::new("missing-synthetic-host"),
            &required,
            Duration::from_secs(1),
            &token,
        )
        .unwrap_err();
        assert_eq!(fault.code, "cancelled");
        let fault = check(
            Path::new("missing-synthetic-host"),
            &required,
            Duration::ZERO,
            &CancellationToken::new(),
        )
        .unwrap_err();
        assert_eq!(fault.code, "runtime_description_timeout");
    }
}
