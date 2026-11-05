//! 轻量宿主只做序列化适配；采集与私有 Job 由平台库唯一实现。
pub(super) use omnitrigger_windows::{
    ProcessCaptureFailure as Failure, ProcessCaptureOptions as Options,
};
use serde::Serialize;

pub(super) fn fault(code: &'static str, started: bool) -> Failure {
    Failure {
        code,
        message: "程序输出采集未完成，请检查错误代码。",
        effect_state: if started { "unknown" } else { "not_started" },
    }
}

#[derive(Debug, Serialize)]
pub(super) struct Captured {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}
pub(super) fn capture(
    program: &str,
    args: &[String],
    options: &Options,
) -> Result<Captured, Failure> {
    omnitrigger_windows::capture_process(program, args, options).map(|value| Captured {
        code: value.code,
        stdout: value.stdout,
        stderr: value.stderr,
    })
}
