//! 独立有界进程采集。既有 run/start 不进入此生命周期。

use serde::Serialize;

#[cfg(any(windows, test))]
pub(super) const MAX_OUTPUT: usize = 64 * 1024;
#[cfg(any(windows, test))]
pub(super) const CHUNK: usize = 4096;

#[derive(Clone, Debug)]
pub(super) struct Options {
    pub timeout_ms: u64,
    #[cfg_attr(not(any(windows, test)), allow(dead_code))] // 非Windows明确拒绝，不执行退出策略。
    pub allow_non_zero: bool,
    pub cwd: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            timeout_ms: 10_000,
            allow_non_zero: false,
            cwd: None,
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct Captured {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Failure {
    pub code: &'static str,
    pub message: &'static str,
    pub effect_state: &'static str,
}

pub(super) fn fault(code: &'static str, started: bool) -> Failure {
    Failure {
        code,
        message: "程序输出采集未完成，请检查错误代码。",
        effect_state: if started { "unknown" } else { "not_started" },
    }
}

fn local_absolute(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
        && !value.contains('\0')
        && !value[3..]
            .split(['\\', '/'])
            .any(|part| matches!(part, "." | ".."))
}

pub(super) fn validate(program: &str, args: &[String], options: &Options) -> Result<(), Failure> {
    // 明确本地绝对 EXE，避免隐式 PATH/cmd/bat 解析；脚本须显式传解释器 EXE。
    if !local_absolute(program)
        || !program.to_ascii_lowercase().ends_with(".exe")
        || args.len() > 128
        || args.iter().any(|arg| arg.contains('\0'))
        || !(1..=60_000).contains(&options.timeout_ms)
        || options
            .cwd
            .as_deref()
            .is_some_and(|cwd| !local_absolute(cwd))
    {
        return Err(fault("invalid_arguments", false));
    }
    if command_line(program, args).len() > 32_767 {
        return Err(fault("invalid_arguments", false));
    }
    Ok(())
}

fn quote(value: &str) -> String {
    let mut result = String::from("\"");
    let mut slashes = 0;
    for character in value.chars() {
        if character == '\\' {
            slashes += 1;
            continue;
        }
        if character == '"' {
            result.push_str(&"\\".repeat(slashes * 2 + 1));
        } else {
            result.push_str(&"\\".repeat(slashes));
        }
        slashes = 0;
        result.push(character);
    }
    result.push_str(&"\\".repeat(slashes * 2));
    result.push('"');
    result
}

pub(super) fn command_line(program: &str, args: &[String]) -> Vec<u16> {
    std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .map(quote)
        .collect::<Vec<_>>()
        .join(" ")
        .encode_utf16()
        .chain([0])
        .collect()
}

#[cfg(any(windows, test))]
#[derive(Debug)]
pub(super) struct ReadChunk {
    pub bytes: Vec<u8>,
    pub eof: bool,
}

#[cfg(any(windows, test))]
pub(super) trait Backend {
    fn start(&mut self, program: &str, args: &[String], options: &Options) -> Result<(), Failure>;
    fn read(&mut self, stderr: bool) -> Result<ReadChunk, Failure>;
    fn exit_code(&mut self) -> Result<Option<i32>, Failure>;
    fn milliseconds(&self) -> u64;
    fn wait(&mut self);
    fn finish(&mut self) -> Result<(), Failure>;
}

#[cfg(any(windows, test))]
pub(super) fn run_with(
    backend: &mut impl Backend,
    program: &str,
    args: &[String],
    options: &Options,
) -> Result<Captured, Failure> {
    validate(program, args, options)?;
    let began = backend.milliseconds();
    let deadline = began
        .checked_add(options.timeout_ms)
        .ok_or_else(|| fault("process_clock_error", false))?;
    backend.start(program, args, options)?;
    let result = (|| {
        let mut last = began;
        let mut output = [Vec::new(), Vec::new()];
        let mut eof = [false, false];
        loop {
            // 每轮分别读取最多 CHUNK 字节；不等一条管道 EOF 才读取另一条。
            for index in 0..2 {
                if eof[index] {
                    continue;
                }
                let chunk = backend.read(index == 1)?;
                if chunk.bytes.len() > CHUNK
                    || output[0].len() + output[1].len() + chunk.bytes.len() > MAX_OUTPUT
                {
                    return Err(fault("process_output_too_large", true));
                }
                output[index].extend_from_slice(&chunk.bytes);
                eof[index] = chunk.eof;
            }
            let code = backend.exit_code()?;
            let now = backend.milliseconds();
            if now < last {
                return Err(fault("process_clock_error", true));
            }
            last = now;
            if now >= deadline {
                return Err(fault("process_timeout", true));
            }
            if let Some(code) = code.filter(|_| eof.iter().all(|ended| *ended)) {
                let stdout = String::from_utf8(std::mem::take(&mut output[0]))
                    .map_err(|_| fault("process_invalid_utf8", true))?;
                let stderr = String::from_utf8(std::mem::take(&mut output[1]))
                    .map_err(|_| fault("process_invalid_utf8", true))?;
                if code != 0 && !options.allow_non_zero {
                    return Err(fault("process_exit_nonzero", true));
                }
                return Ok(Captured {
                    code,
                    stdout,
                    stderr,
                });
            }
            backend.wait();
        }
    })();
    // 不交付部分输出。包括退出后仍持管道的后代在内，都须完成 Job 收尾。
    backend.finish()?;
    result
}

#[cfg(windows)]
#[path = "process_capture_windows.rs"]
mod native;

pub(super) fn capture(
    program: &str,
    args: &[String],
    options: &Options,
) -> Result<Captured, Failure> {
    validate(program, args, options)?;
    #[cfg(windows)]
    {
        run_with(&mut native::Native::default(), program, args, options)
    }
    #[cfg(not(windows))]
    {
        Err(fault("unsupported_platform", false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct Fake {
        now: u64,
        starts: usize,
        finishes: usize,
        reads: Vec<bool>,
        out: VecDeque<ReadChunk>,
        err: VecDeque<ReadChunk>,
        code: i32,
        advance: bool,
        fail_read: bool,
        fail_finish: bool,
        running: bool,
    }
    impl Default for Fake {
        fn default() -> Self {
            Self {
                now: 0,
                starts: 0,
                finishes: 0,
                reads: vec![],
                out: VecDeque::new(),
                err: VecDeque::new(),
                code: 0,
                advance: true,
                fail_read: false,
                fail_finish: false,
                running: false,
            }
        }
    }
    impl Backend for Fake {
        fn start(&mut self, _: &str, _: &[String], _: &Options) -> Result<(), Failure> {
            self.starts += 1;
            Ok(())
        }
        fn read(&mut self, stderr: bool) -> Result<ReadChunk, Failure> {
            self.reads.push(stderr);
            if self.fail_read {
                return Err(fault("process_read_failed", true));
            }
            Ok((if stderr { &mut self.err } else { &mut self.out })
                .pop_front()
                .unwrap_or(ReadChunk {
                    bytes: vec![],
                    eof: true,
                }))
        }
        fn exit_code(&mut self) -> Result<Option<i32>, Failure> {
            Ok(if self.running { None } else { Some(self.code) })
        }
        fn milliseconds(&self) -> u64 {
            self.now
        }
        fn wait(&mut self) {
            if self.advance {
                self.now += 1;
            } else {
                self.now = self.now.saturating_sub(1);
            }
        }
        fn finish(&mut self) -> Result<(), Failure> {
            self.finishes += 1;
            if self.fail_finish {
                Err(fault("process_cleanup_failed", true))
            } else {
                Ok(())
            }
        }
    }
    fn execute(fake: &mut Fake, options: Options) -> Result<Captured, Failure> {
        run_with(fake, "C:\\fixture.exe", &[], &options)
    }

    #[test]
    fn validation_precedes_start_and_preserves_original_run_contract() {
        for program in [
            "python",
            "C:\\x.cmd",
            "C:\\x.bat",
            "\\\\server\\x.exe",
            "C:\\..\\x.exe",
            "",
        ] {
            let mut fake = Fake::default();
            assert_eq!(
                run_with(&mut fake, program, &[], &Options::default())
                    .unwrap_err()
                    .code,
                "invalid_arguments"
            );
            assert_eq!((fake.starts, fake.finishes), (0, 0));
        }
        for timeout_ms in [0, 60001] {
            let mut fake = Fake::default();
            assert!(execute(
                &mut fake,
                Options {
                    timeout_ms,
                    ..Options::default()
                }
            )
            .is_err());
            assert_eq!(fake.starts, 0);
        }
    }

    #[test]
    fn command_arguments_are_quoted_without_shell_or_loss() {
        assert_eq!(quote(""), "\"\"");
        assert_eq!(quote("a b"), "\"a b\"");
        assert_eq!(quote("a\"b"), "\"a\\\"b\"");
        assert_eq!(quote("a\\"), "\"a\\\\\"");
        assert_eq!(quote("&echo %PATH%"), "\"&echo %PATH%\"");
    }

    #[test]
    fn pipes_are_polled_fairly_and_utf8_chunks_are_joined_before_decoding() {
        let mut fake = Fake::default();
        fake.out.extend([
            ReadChunk {
                bytes: vec![0xe4, 0xb8],
                eof: false,
            },
            ReadChunk {
                bytes: vec![0xad],
                eof: true,
            },
        ]);
        fake.err.push_back(ReadChunk {
            bytes: b"stderr".to_vec(),
            eof: true,
        });
        let result = execute(&mut fake, Options::default()).unwrap();
        assert_eq!(result.stdout, "中");
        assert_eq!(result.stderr, "stderr");
        assert_eq!(fake.reads, [false, true, false]);
        assert_eq!(fake.finishes, 1);
    }

    #[test]
    fn aggregate_limit_is_shared_and_no_partial_receipt_is_returned() {
        let mut fake = Fake::default();
        for _ in 0..8 {
            fake.out.push_back(ReadChunk {
                bytes: vec![b'a'; CHUNK],
                eof: false,
            });
            fake.err.push_back(ReadChunk {
                bytes: vec![b'b'; CHUNK],
                eof: false,
            });
        }
        let result = execute(&mut fake, Options::default()).unwrap();
        assert_eq!(result.stdout.len() + result.stderr.len(), MAX_OUTPUT);
        let mut fake = Fake::default();
        for _ in 0..9 {
            fake.out.push_back(ReadChunk {
                bytes: vec![b'x'; CHUNK],
                eof: false,
            });
            fake.err.push_back(ReadChunk {
                bytes: vec![b'y'; CHUNK],
                eof: false,
            });
        }
        assert_eq!(
            execute(&mut fake, Options::default()).unwrap_err().code,
            "process_output_too_large"
        );
        assert_eq!(fake.finishes, 1);
    }

    #[test]
    fn invalid_utf8_and_nonzero_are_explicit_and_always_cleaned() {
        let mut fake = Fake::default();
        fake.out.push_back(ReadChunk {
            bytes: vec![0xff],
            eof: true,
        });
        assert_eq!(
            execute(&mut fake, Options::default()).unwrap_err().code,
            "process_invalid_utf8"
        );
        assert_eq!(fake.finishes, 1);
        let mut fake = Fake {
            code: 7,
            ..Fake::default()
        };
        assert_eq!(
            execute(&mut fake, Options::default()).unwrap_err().code,
            "process_exit_nonzero"
        );
        assert_eq!(fake.finishes, 1);
        let mut fake = Fake {
            code: 7,
            ..Fake::default()
        };
        assert_eq!(
            execute(
                &mut fake,
                Options {
                    allow_non_zero: true,
                    ..Options::default()
                }
            )
            .unwrap()
            .code,
            7
        );
    }

    #[test]
    fn descendant_holding_pipe_cannot_turn_exit_into_an_unbounded_wait() {
        let mut fake = Fake::default();
        for _ in 0..8 {
            fake.out.push_back(ReadChunk {
                bytes: vec![],
                eof: false,
            });
        }
        assert_eq!(
            execute(
                &mut fake,
                Options {
                    timeout_ms: 3,
                    ..Options::default()
                }
            )
            .unwrap_err()
            .code,
            "process_timeout"
        );
        assert_eq!(fake.finishes, 1);
        let mut fake = Fake {
            now: u64::MAX,
            ..Fake::default()
        };
        assert_eq!(
            execute(&mut fake, Options::default()).unwrap_err().code,
            "process_clock_error"
        );
        assert_eq!(fake.starts, 0);
        let mut fake = Fake {
            running: true,
            ..Fake::default()
        };
        assert_eq!(
            execute(
                &mut fake,
                Options {
                    timeout_ms: 1,
                    ..Options::default()
                }
            )
            .unwrap_err()
            .code,
            "process_timeout"
        );
        assert_eq!(fake.finishes, 1);
    }

    #[test]
    fn read_failure_backward_clock_and_cleanup_failure_never_return_output() {
        let mut fake = Fake {
            fail_read: true,
            ..Fake::default()
        };
        assert_eq!(
            execute(&mut fake, Options::default()).unwrap_err().code,
            "process_read_failed"
        );
        assert_eq!(fake.finishes, 1);
        let mut fake = Fake {
            now: 5,
            advance: false,
            ..Fake::default()
        };
        fake.out.push_back(ReadChunk {
            bytes: vec![],
            eof: false,
        });
        assert_eq!(
            execute(&mut fake, Options::default()).unwrap_err().code,
            "process_clock_error"
        );
        assert_eq!(fake.finishes, 1);
        let mut fake = Fake {
            fail_finish: true,
            ..Fake::default()
        };
        assert_eq!(
            execute(&mut fake, Options::default()).unwrap_err().code,
            "process_cleanup_failed"
        );
        assert_eq!(fake.finishes, 1);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "需要编译自有夹具并设置OMNITRIGGER_CAPTURE_FIXTURE，显式串行执行"]
    fn controlled_fixture_capture_cases() {
        let fixture = std::env::var("OMNITRIGGER_CAPTURE_FIXTURE").expect("需要明确的自有夹具EXE");
        let arguments = |values: &[&str]| {
            values
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>()
        };
        let result = capture(
            &fixture,
            &arguments(&["dual", "32768", "32768"]),
            &Options::default(),
        )
        .unwrap();
        assert_eq!(result.stdout.len() + result.stderr.len(), MAX_OUTPUT);
        assert_eq!(
            capture(
                &fixture,
                &arguments(&["dual", "32769", "32768"]),
                &Options::default()
            )
            .unwrap_err()
            .code,
            "process_output_too_large"
        );
        assert_eq!(
            capture(&fixture, &arguments(&["invalid-utf8"]), &Options::default())
                .unwrap_err()
                .code,
            "process_invalid_utf8"
        );
        assert_eq!(
            capture(&fixture, &arguments(&["exit", "7"]), &Options::default())
                .unwrap_err()
                .code,
            "process_exit_nonzero"
        );
        assert_eq!(
            capture(
                &fixture,
                &arguments(&["exit", "7"]),
                &Options {
                    allow_non_zero: true,
                    ..Options::default()
                }
            )
            .unwrap()
            .code,
            7
        );
        assert_eq!(
            capture(
                &fixture,
                &arguments(&["sleep", "1000"]),
                &Options {
                    timeout_ms: 10,
                    ..Options::default()
                }
            )
            .unwrap_err()
            .code,
            "process_timeout"
        );
    }
}
