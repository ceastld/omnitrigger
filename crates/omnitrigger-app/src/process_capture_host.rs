//! 显式 process.capture 的 NativeHost 适配。没有目标窗口依赖，不包装或改写旧 run。
use omnitrigger_actions::{ActionError, CancellationToken, NativeHost};
use omnitrigger_windows::{
    capture_process_if, validate_process_capture, ProcessCaptureControl as Control,
    ProcessCaptureFailure as Failure, ProcessCaptureOptions as Options,
    ProcessCaptureResult as Captured,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Instant;

const METHOD: &str = "process.capture";
fn timeout() -> u64 {
    10_000
}
fn cwd<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    program: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default = "timeout")]
    timeout_ms: u64,
    #[serde(default)]
    allow_non_zero: bool,
    #[serde(default, deserialize_with = "cwd")]
    cwd: Option<String>,
}
fn fault(code: &str, effect: &str) -> ActionError {
    let mut error = ActionError::new(code, "程序输出采集未完成，请检查错误代码。");
    error.effect_state = Some(effect.into());
    error.method = Some(METHOD.into());
    error
}
fn request(value: Value) -> Result<(String, Vec<String>, Options), ActionError> {
    let value: Request =
        serde_json::from_value(value).map_err(|_| fault("invalid_input", "not_started"))?;
    if value.args.iter().any(|arg| {
        arg == "--omni-selected-text-once"
            || arg.starts_with("--omni-selected-text-once=")
            || arg == "--omni-explorer-context-once"
            || arg.starts_with("--omni-explorer-context-once=")
    }) {
        return Err(fault("invalid_input", "not_started"));
    }
    let options = Options {
        timeout_ms: value.timeout_ms,
        allow_non_zero: value.allow_non_zero,
        cwd: value.cwd,
    };
    validate_process_capture(&value.program, &value.args, &options)
        .map_err(|failure| fault(failure.code, failure.effect_state))?;
    Ok((value.program, value.args, options))
}
fn stopped(control: Control, started: bool) -> Result<(), ActionError> {
    let effect = if started { "unknown" } else { "not_started" };
    match control {
        Control::Continue => Ok(()),
        Control::Cancelled => Err(fault("cancelled", effect)),
        Control::Deadline => Err(fault("timeout", effect)),
    }
}
fn execute_with(
    params: Value,
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
    capture: impl FnOnce(
        &str,
        &[String],
        &Options,
        &mut dyn FnMut() -> Control,
    ) -> Result<Captured, Failure>,
) -> Result<Value, ActionError> {
    let (program, args, options) = request(params)?;
    let mut check = || {
        if cancellation.is_cancelled() {
            Control::Cancelled
        } else if deadline.is_some_and(|limit| Instant::now() >= limit) {
            Control::Deadline
        } else {
            Control::Continue
        }
    };
    stopped(check(), false)?;
    let value = capture(&program, &args, &options, &mut check)
        .map_err(|failure| fault(failure.code, failure.effect_state))?;
    stopped(check(), true)?;
    Ok(json!({"code":value.code,"stdout":value.stdout,"stderr":value.stderr}))
}
fn execute(
    params: Value,
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
) -> Result<Value, ActionError> {
    execute_with(
        params,
        cancellation,
        deadline,
        |program, args, options, check| capture_process_if(program, args, options, check),
    )
}

pub struct WithProcessCapture<H> {
    inner: H,
}
impl<H> WithProcessCapture<H> {
    pub fn new(inner: H) -> Self {
        Self { inner }
    }
}
impl<H: NativeHost> NativeHost for WithProcessCapture<H> {
    fn call(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError> {
        if method == METHOD {
            execute(params, cancellation, None)
        } else {
            self.inner.call(method, params, cancellation)
        }
    }
    fn call_with_deadline(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<Value, ActionError> {
        if method == METHOD {
            execute(params, cancellation, Some(deadline))
        } else {
            self.inner
                .call_with_deadline(method, params, cancellation, deadline)
        }
    }
}

#[cfg(all(test, windows))]
#[path = "process_capture_node_native_tests.rs"]
mod node_native;

#[cfg(test)]
mod tests {
    use super::*;
    const PAYLOAD: &str = "PRIVATE_SYNTHETIC_PAYLOAD";
    fn input() -> Value {
        json!({"program":"C:/Owned/capture.exe"})
    }
    fn success() -> Captured {
        Captured {
            code: 0,
            stdout: "合成".into(),
            stderr: String::new(),
        }
    }
    fn assert_fault(error: ActionError, code: &str, effect: &str) {
        assert_eq!(error.code, code);
        assert_eq!(error.effect_state.as_deref(), Some(effect));
        assert_eq!(error.method.as_deref(), Some(METHOD));
        assert!(error.required_permissions.is_empty());
        assert!(!error.message.contains(PAYLOAD));
    }
    #[test]
    fn strict_request_defaults_and_unknown_fields_do_not_start_capture() {
        let (program, args, options) = request(input()).unwrap();
        assert_eq!(program, "C:/Owned/capture.exe");
        assert!(args.is_empty());
        assert_eq!(options.timeout_ms, 10_000);
        assert!(!options.allow_non_zero);
        assert!(options.cwd.is_none());
        for bad in [
            json!({"program":PAYLOAD}),
            json!({"program":"C:/Owned/capture.exe","cwd":null}),
            json!({"program":"C:/Owned/capture.exe","shell":true}),
            json!({"program":"C:/Owned/capture.exe","args":[1]}),
        ] {
            let error = execute_with(bad, &CancellationToken::new(), None, |_, _, _, _| {
                panic!("不得启动")
            })
            .unwrap_err();
            assert_eq!(error.effect_state.as_deref(), Some("not_started"));
            assert!(!error.message.contains(PAYLOAD));
        }
    }
    #[test]
    fn cancelled_or_expired_before_dispatch_does_not_invoke_capture() {
        let token = CancellationToken::new();
        token.cancel();
        assert_fault(
            execute_with(input(), &token, None, |_, _, _, _| panic!("不得启动")).unwrap_err(),
            "cancelled",
            "not_started",
        );
        assert_fault(
            execute_with(
                input(),
                &CancellationToken::new(),
                Some(Instant::now()),
                |_, _, _, _| panic!("不得启动"),
            )
            .unwrap_err(),
            "timeout",
            "not_started",
        );
    }
    #[test]
    fn explicit_options_and_outer_control_reach_the_same_capture_engine() {
        let token = CancellationToken::new();
        let value=execute_with(json!({"program":"C:/Owned/capture.exe","args":["a b"],"timeout_ms":60000,"allow_non_zero":true,"cwd":"C:/Owned"}),
            &token,None,|program,args,options,check| {
                assert_eq!(program,"C:/Owned/capture.exe"); assert_eq!(args,["a b"]);
                assert_eq!(options.timeout_ms,60000);assert!(options.allow_non_zero);assert_eq!(options.cwd.as_deref(),Some("C:/Owned"));
                assert_eq!(check(),Control::Continue);Ok(success())
            }).unwrap();
        assert_eq!(value, json!({"code":0,"stdout":"合成","stderr":""}));
    }
    #[test]
    fn cancellation_during_capture_or_after_cleanup_never_delivers_output() {
        let token = CancellationToken::new();
        assert_fault(
            execute_with(input(), &token, None, |_, _, _, check| {
                token.cancel();
                assert_eq!(check(), Control::Cancelled);
                Ok(success())
            })
            .unwrap_err(),
            "cancelled",
            "unknown",
        );
    }
    #[test]
    fn cleanup_failure_is_not_replaced_by_later_cancellation() {
        let token = CancellationToken::new();
        assert_fault(
            execute_with(input(), &token, None, |_, _, _, _| {
                token.cancel();
                Err(Failure {
                    code: "process_cleanup_failed",
                    message: PAYLOAD,
                    effect_state: "unknown",
                })
            })
            .unwrap_err(),
            "process_cleanup_failed",
            "unknown",
        );
    }
    #[test]
    fn other_methods_keep_their_deadline_and_existing_host_semantics() {
        struct Original;
        impl NativeHost for Original {
            fn call(
                &self,
                method: &str,
                params: Value,
                _: &CancellationToken,
            ) -> Result<Value, ActionError> {
                Ok(json!({"method":method,"params":params}))
            }
            fn call_with_deadline(
                &self,
                method: &str,
                _: Value,
                _: &CancellationToken,
                deadline: Instant,
            ) -> Result<Value, ActionError> {
                assert_eq!(method, "existing.method");
                assert!(Instant::now() < deadline);
                Ok(json!({"forwarded":true}))
            }
        }
        let host = WithProcessCapture::new(Original);
        let token = CancellationToken::new();
        assert_eq!(
            host.call("existing.method", json!({"value":1}), &token)
                .unwrap()["params"]["value"],
            1
        );
        assert_eq!(
            host.call_with_deadline(
                "existing.method",
                json!({}),
                &token,
                Instant::now() + std::time::Duration::from_secs(10)
            )
            .unwrap(),
            json!({"forwarded":true})
        );
    }
}

#[cfg(test)]
mod reserved_selection_helper_tests {
    use super::*;
    #[test]
    fn scripts_cannot_request_the_internal_helper_via_capture() {
        for flag in [
            "--omni-selected-text-once",
            "--omni-selected-text-once={}",
            "--omni-explorer-context-once",
            "--omni-explorer-context-once={}",
        ] {
            let error = request(json!({"program":"C:/Owned/omnitrigger.exe","args":[flag,"{}"]}))
                .unwrap_err();
            assert_eq!(error.code, "invalid_input");
            assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        }
    }
}
