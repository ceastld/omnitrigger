//! 选区复制的有界观察流程。只在动作工作线程调用；不恢复剪贴板、不重发复制。
#[cfg(any(windows, test))]
use crate::clipboard_data::ClipboardDataError;
use crate::clipboard_data::{ClipboardDataFormat, ClipboardDataSnapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionCopyOptions {
    pub format: ClipboardDataFormat,
    pub timeout_ms: u32,
    pub poll_interval_ms: u32,
    pub settle_ms: u32,
    pub allow_unchanged: bool,
}

impl Default for SelectionCopyOptions {
    fn default() -> Self {
        Self {
            format: ClipboardDataFormat::Auto,
            timeout_ms: 600,
            poll_interval_ms: 15,
            settle_ms: 30,
            allow_unchanged: false,
        }
    }
}

impl SelectionCopyOptions {
    /// 调用方必须在访问目标、剪贴板或输入前校验；等待预算不含阻塞系统调用。
    pub fn validate(&self) -> Result<(), SelectionDataError> {
        if !(1..=5000).contains(&self.timeout_ms)
            || !(1..=1000).contains(&self.poll_interval_ms)
            || self.poll_interval_ms > self.timeout_ms
            || self.settle_ms > 1000
        {
            return Err(SelectionDataError::new(
                "invalid_arguments",
                "复制等待应为 1–5000ms，轮询应为 1–1000ms 且不超过等待时间，稳定等待应为 0–1000ms。",
                "not_started",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionDataSnapshot {
    pub clipboard: ClipboardDataSnapshot,
    pub changed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionDataError {
    pub code: &'static str,
    pub message: String,
    pub effect_state: &'static str,
}

impl SelectionDataError {
    pub(crate) fn new(
        code: &'static str,
        message: impl Into<String>,
        effect_state: &'static str,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            effect_state,
        }
    }

    #[cfg(any(windows, test))]
    fn at(mut self, effect_state: &'static str) -> Self {
        self.effect_state = effect_state;
        self
    }
}

impl std::fmt::Display for SelectionDataError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SelectionDataError {}

// 后端提供显式单调时间；流程不读隐含时钟，也不解释剪贴板格式。
#[cfg(any(windows, test))]
trait Backend {
    fn check(&mut self) -> Result<(), SelectionDataError>;
    fn now_ms(&mut self) -> Result<u64, SelectionDataError>;
    /// 在输入门闩内完成最终核验、before采样和唯一一次复制。
    /// 后端必须把提交请求后的失败保守标记为unknown，前置拒绝标记为not_started。
    fn copy_once(&mut self) -> Result<u32, SelectionDataError>;
    fn sequence_number(&mut self) -> u32;
    fn wait_ms(&mut self, ms: u32);
    fn read_data(
        &mut self,
        format: ClipboardDataFormat,
    ) -> Result<ClipboardDataSnapshot, ClipboardDataError>;
}

#[cfg(any(windows, test))]
#[derive(Default)]
struct Clock {
    last: Option<u64>,
}

#[cfg(any(windows, test))]
fn clock_error(effect: &'static str) -> SelectionDataError {
    SelectionDataError::new(
        "selection_clock_invalid",
        "复制等待的单调时间倒退、溢出或未推进。",
        effect,
    )
}

#[cfg(any(windows, test))]
impl Clock {
    fn sample(
        &mut self,
        backend: &mut impl Backend,
        effect: &'static str,
    ) -> Result<u64, SelectionDataError> {
        let now = backend.now_ms().map_err(|error| error.at(effect))?;
        if self.last.is_some_and(|last| now < last) {
            return Err(clock_error(effect));
        }
        self.last = Some(now);
        Ok(now)
    }

    fn wait(&mut self, backend: &mut impl Backend, ms: u32) -> Result<(), SelectionDataError> {
        backend.check().map_err(|error| error.at("unknown"))?;
        let before = self.sample(backend, "unknown")?;
        backend.wait_ms(ms);
        backend.check().map_err(|error| error.at("unknown"))?;
        if self.sample(backend, "unknown")? <= before {
            return Err(clock_error("unknown"));
        }
        Ok(())
    }
}

#[cfg(any(windows, test))]
fn observed_sequence(backend: &mut impl Backend) -> Result<u32, SelectionDataError> {
    backend.check().map_err(|error| error.at("unknown"))?;
    let sequence = backend.sequence_number();
    backend.check().map_err(|error| error.at("unknown"))?;
    Ok(sequence)
}

#[cfg(any(windows, test))]
fn unchanged() -> SelectionDataError {
    SelectionDataError::new(
        "selection_unchanged",
        "复制等待结束后剪贴板序号未变化。",
        "unknown",
    )
}

#[cfg(any(windows, test))]
fn copy_with_backend(
    options: SelectionCopyOptions,
    backend: &mut impl Backend,
) -> Result<SelectionDataSnapshot, SelectionDataError> {
    options.validate()?;
    backend.check().map_err(|error| error.at("not_started"))?;
    let mut clock = Clock::default();
    clock.sample(backend, "not_started")?;
    let before = backend.copy_once()?;
    backend.check().map_err(|error| error.at("unknown"))?;
    let started = clock.sample(backend, "unknown")?;
    let deadline = started
        .checked_add(u64::from(options.timeout_ms))
        .ok_or_else(|| clock_error("unknown"))?;
    loop {
        // 首次立即观察，每次等待之后再次观察，也包括deadline处的最终观察。
        let sequence = observed_sequence(backend)?;
        let now = clock.sample(backend, "unknown")?;
        if sequence != before {
            break;
        }
        if now >= deadline {
            if !options.allow_unchanged {
                return Err(unchanged());
            }
            break;
        }
        let wait = u64::from(options.poll_interval_ms).min(deadline - now) as u32;
        clock.wait(backend, wait)?;
    }
    if options.settle_ms > 0 {
        let settled = clock
            .sample(backend, "unknown")?
            .checked_add(u64::from(options.settle_ms))
            .ok_or_else(|| clock_error("unknown"))?;
        loop {
            let now = clock.sample(backend, "unknown")?;
            if now >= settled {
                break;
            }
            let wait = u64::from(options.poll_interval_ms).min(settled - now) as u32;
            clock.wait(backend, wait)?;
        }
    }
    backend.check().map_err(|error| error.at("unknown"))?;
    let result = backend.read_data(options.format);
    backend.check().map_err(|error| error.at("unknown"))?;
    let clipboard =
        result.map_err(|error| SelectionDataError::new(error.code, error.message, "unknown"))?;
    // 最终读取的序号才决定changed，不借用较早轮询的变化，也不证明数据生产者。
    let changed = clipboard.sequence_number != before;
    if !changed && !options.allow_unchanged {
        return Err(unchanged());
    }
    Ok(SelectionDataSnapshot { clipboard, changed })
}

#[cfg(windows)]
pub(crate) fn copy_selection_data_checked(
    target: crate::ShortcutTarget,
    options: SelectionCopyOptions,
    check: impl FnMut() -> Result<(), SelectionDataError>,
) -> Result<SelectionDataSnapshot, SelectionDataError> {
    options.validate()?;
    copy_with_backend(
        options,
        &mut native::WindowsBackend {
            target,
            check,
            started: std::time::Instant::now(),
        },
    )
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::time::{Duration, Instant};

    pub(super) struct WindowsBackend<C> {
        pub(super) target: crate::ShortcutTarget,
        pub(super) check: C,
        pub(super) started: Instant,
    }

    impl<C: FnMut() -> Result<(), SelectionDataError>> Backend for WindowsBackend<C> {
        fn check(&mut self) -> Result<(), SelectionDataError> {
            (self.check)()
        }
        fn now_ms(&mut self) -> Result<u64, SelectionDataError> {
            u64::try_from(self.started.elapsed().as_millis())
                .map_err(|_| clock_error("not_started"))
        }
        fn copy_once(&mut self) -> Result<u32, SelectionDataError> {
            use crate::shortcut::TryShortcutError;
            let target = self.target;
            let mut before = 0;
            let mut possible_submission = false;
            let mut guard_error = None;
            let sent = crate::shortcut::try_send_shortcut_checked_before_send(
                &[0xa2, 0x43],
                target,
                || {
                    self.check().map_err(|error| {
                        guard_error = Some(error);
                        "复制调用检查未通过。".into()
                    })
                },
                || {
                    // 门闩内、既有输入计划完成后、提交请求前采样；不宣称原子归属。
                    before = crate::clipboard_sequence_number();
                    possible_submission = true;
                },
            );
            if let Err(error) = sent {
                if let Some(error) = guard_error {
                    return Err(error.at("not_started"));
                }
                return Err(match error {
                    TryShortcutError::Busy => SelectionDataError::new(
                        "selection_busy",
                        "另一次输入提交尚未结束，请稍后再试。",
                        "not_started",
                    ),
                    TryShortcutError::Unavailable => SelectionDataError::new(
                        "selection_input_unavailable",
                        "复制输入串行状态不可用。",
                        "not_started",
                    ),
                    TryShortcutError::Failed => SelectionDataError::new(
                        "selection_input_failed",
                        "复制输入未完成，目标结果和按键状态需要检查。",
                        if possible_submission {
                            "unknown"
                        } else {
                            "not_started"
                        },
                    ),
                });
            }
            Ok(before)
        }
        fn sequence_number(&mut self) -> u32 {
            crate::clipboard_sequence_number()
        }
        fn wait_ms(&mut self, ms: u32) {
            std::thread::sleep(Duration::from_millis(u64::from(ms)));
        }
        fn read_data(
            &mut self,
            format: ClipboardDataFormat,
        ) -> Result<ClipboardDataSnapshot, ClipboardDataError> {
            crate::clipboard_data::read_clipboard_data_format(format)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard_data::ClipboardData;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Operation {
        Copy,
        Sequence,
        Wait,
        Read,
    }

    struct Fake {
        now: u64,
        before: u32,
        changes: Vec<(u64, u32)>,
        final_sequence: Option<u32>,
        data: ClipboardData,
        calls: Vec<&'static str>,
        waits: Vec<u32>,
        read_formats: Vec<ClipboardDataFormat>,
        last: Option<Operation>,
        fail_after: Option<Operation>,
        guard_code: &'static str,
        initial_guard_failure: bool,
        copy_error: Option<SelectionDataError>,
        read_error: Option<ClipboardDataError>,
        clock_error: bool,
        wait_shift: i64,
    }

    impl Default for Fake {
        fn default() -> Self {
            Self {
                now: 0,
                before: 9,
                changes: Vec::new(),
                final_sequence: None,
                data: ClipboardData::Text("".into()),
                calls: Vec::new(),
                waits: Vec::new(),
                read_formats: Vec::new(),
                last: None,
                fail_after: None,
                guard_code: "operation_guard_failed",
                initial_guard_failure: false,
                copy_error: None,
                read_error: None,
                clock_error: false,
                wait_shift: 0,
            }
        }
    }

    impl Fake {
        fn current_sequence(&self) -> u32 {
            self.changes
                .iter()
                .rev()
                .find(|(at, _)| *at <= self.now)
                .map_or(self.before, |(_, sequence)| *sequence)
        }
        fn changed_immediately() -> Self {
            Self {
                changes: vec![(0, 10)],
                ..Self::default()
            }
        }
    }

    impl Backend for Fake {
        fn check(&mut self) -> Result<(), SelectionDataError> {
            self.calls.push("check");
            if self.initial_guard_failure
                || (self.fail_after.is_some() && self.fail_after == self.last)
            {
                return Err(SelectionDataError::new(
                    self.guard_code,
                    "固定测试检查错误",
                    "not_started",
                ));
            }
            Ok(())
        }
        fn now_ms(&mut self) -> Result<u64, SelectionDataError> {
            self.calls.push("clock");
            if self.clock_error {
                return Err(super::clock_error("not_started"));
            }
            Ok(self.now)
        }
        fn copy_once(&mut self) -> Result<u32, SelectionDataError> {
            self.calls.push("copy");
            self.last = Some(Operation::Copy);
            if let Some(error) = self.copy_error.take() {
                return Err(error);
            }
            Ok(self.before)
        }
        fn sequence_number(&mut self) -> u32 {
            self.calls.push("sequence");
            self.last = Some(Operation::Sequence);
            self.current_sequence()
        }
        fn wait_ms(&mut self, ms: u32) {
            self.calls.push("wait");
            self.last = Some(Operation::Wait);
            self.waits.push(ms);
            self.now = self
                .now
                .checked_add_signed(i64::from(ms) + self.wait_shift)
                .unwrap();
        }
        fn read_data(
            &mut self,
            format: ClipboardDataFormat,
        ) -> Result<ClipboardDataSnapshot, ClipboardDataError> {
            self.read_formats.push(format);
            self.calls.push("read");
            self.last = Some(Operation::Read);
            if let Some(error) = self.read_error.take() {
                return Err(error);
            }
            Ok(ClipboardDataSnapshot {
                data: std::mem::replace(&mut self.data, ClipboardData::Text(String::new())),
                sequence_number: self
                    .final_sequence
                    .unwrap_or_else(|| self.current_sequence()),
            })
        }
    }

    fn options() -> SelectionCopyOptions {
        SelectionCopyOptions {
            format: ClipboardDataFormat::Auto,
            timeout_ms: 10,
            poll_interval_ms: 4,
            settle_ms: 0,
            allow_unchanged: false,
        }
    }

    fn assert_error(
        result: Result<SelectionDataSnapshot, SelectionDataError>,
        code: &str,
        effect: &str,
    ) {
        let error = result.unwrap_err();
        assert_eq!(error.code, code);
        assert_eq!(error.effect_state, effect);
    }

    #[test]
    fn requested_format_reaches_final_read_without_extra_copy_or_sequence_change() {
        for format in [
            ClipboardDataFormat::Auto,
            ClipboardDataFormat::Text,
            ClipboardDataFormat::Files,
        ] {
            let mut fake = Fake::changed_immediately();
            let snapshot = copy_with_backend(
                SelectionCopyOptions {
                    format,
                    ..options()
                },
                &mut fake,
            )
            .unwrap();
            assert!(snapshot.changed);
            assert_eq!(snapshot.clipboard.sequence_number, 10);
            assert_eq!(fake.read_formats, [format]);
            assert_eq!(fake.calls.iter().filter(|call| **call == "copy").count(), 1);
        }
    }

    #[test]
    fn options_defaults_and_every_invalid_boundary_are_checked_before_backend_access() {
        let defaults = SelectionCopyOptions::default();
        assert_eq!(
            defaults,
            SelectionCopyOptions {
                format: ClipboardDataFormat::Auto,
                timeout_ms: 600,
                poll_interval_ms: 15,
                settle_ms: 30,
                allow_unchanged: false,
            }
        );
        assert!(defaults.validate().is_ok());
        for invalid in [
            SelectionCopyOptions {
                timeout_ms: 0,
                ..options()
            },
            SelectionCopyOptions {
                timeout_ms: 5001,
                ..options()
            },
            SelectionCopyOptions {
                poll_interval_ms: 0,
                ..options()
            },
            SelectionCopyOptions {
                timeout_ms: 5000,
                poll_interval_ms: 1001,
                ..options()
            },
            SelectionCopyOptions {
                poll_interval_ms: 11,
                ..options()
            },
            SelectionCopyOptions {
                settle_ms: 1001,
                ..options()
            },
        ] {
            let mut fake = Fake::default();
            assert_error(
                copy_with_backend(invalid, &mut fake),
                "invalid_arguments",
                "not_started",
            );
            assert!(fake.calls.is_empty());
        }
        for valid in [
            SelectionCopyOptions {
                timeout_ms: 1,
                poll_interval_ms: 1,
                settle_ms: 0,
                allow_unchanged: true,
                format: ClipboardDataFormat::Auto,
            },
            SelectionCopyOptions {
                timeout_ms: 5000,
                poll_interval_ms: 1000,
                settle_ms: 1000,
                allow_unchanged: false,
                format: ClipboardDataFormat::Auto,
            },
        ] {
            assert!(valid.validate().is_ok());
        }
    }

    #[test]
    fn first_observation_is_immediate_and_empty_text_remains_supported_data() {
        let mut fake = Fake::changed_immediately();
        let snapshot = copy_with_backend(options(), &mut fake).unwrap();
        assert!(snapshot.changed);
        assert_eq!(snapshot.clipboard.sequence_number, 10);
        assert!(matches!(snapshot.clipboard.data, ClipboardData::Text(text) if text.is_empty()));
        assert!(fake.waits.is_empty());
        assert_eq!(fake.calls.iter().filter(|call| **call == "copy").count(), 1);
        assert_eq!(
            fake.calls
                .iter()
                .filter(|call| **call == "sequence")
                .count(),
            1
        );
    }

    #[test]
    fn polls_at_intervals_and_checks_deadline_once_more_without_oversleep() {
        let mut fake = Fake {
            changes: vec![(10, 10)],
            ..Fake::default()
        };
        assert!(copy_with_backend(options(), &mut fake).unwrap().changed);
        assert_eq!(fake.waits, [4, 4, 2]);
        assert_eq!(fake.now, 10);
        assert_eq!(
            fake.calls
                .iter()
                .filter(|call| **call == "sequence")
                .count(),
            4
        );
        assert_eq!(fake.calls.iter().filter(|call| **call == "copy").count(), 1);
    }

    #[test]
    fn unchanged_without_permission_never_reads_current_payload_or_repeats_copy() {
        let mut fake = Fake::default();
        assert_error(
            copy_with_backend(options(), &mut fake),
            "selection_unchanged",
            "unknown",
        );
        assert_eq!(fake.waits, [4, 4, 2]);
        assert!(!fake.calls.contains(&"read"));
        assert_eq!(fake.calls.iter().filter(|call| **call == "copy").count(), 1);
    }

    #[test]
    fn allow_unchanged_reads_current_file_list_and_reports_false() {
        let mut fake = Fake {
            data: ClipboardData::Files(vec!["C:\\sample.txt".into()]),
            ..Fake::default()
        };
        let snapshot = copy_with_backend(
            SelectionCopyOptions {
                allow_unchanged: true,
                ..options()
            },
            &mut fake,
        )
        .unwrap();
        assert!(!snapshot.changed);
        assert!(
            matches!(snapshot.clipboard.data, ClipboardData::Files(paths) if paths == ["C:\\sample.txt"])
        );
        assert_eq!(fake.waits, [4, 4, 2]);
    }

    #[test]
    fn final_reader_snapshot_overrides_earlier_sequence_observation() {
        for allowed in [false, true] {
            let mut fake = Fake {
                final_sequence: Some(9),
                ..Fake::changed_immediately()
            };
            let result = copy_with_backend(
                SelectionCopyOptions {
                    allow_unchanged: allowed,
                    ..options()
                },
                &mut fake,
            );
            if allowed {
                assert!(!result.unwrap().changed);
            } else {
                assert_error(result, "selection_unchanged", "unknown");
            }
        }
        let mut fake = Fake {
            final_sequence: Some(12),
            ..Fake::default()
        };
        assert!(
            copy_with_backend(
                SelectionCopyOptions {
                    allow_unchanged: true,
                    ..options()
                },
                &mut fake
            )
            .unwrap()
            .changed
        );
    }

    #[test]
    fn sequence_wraparound_uses_inequality_instead_of_ordering() {
        let mut fake = Fake {
            before: u32::MAX,
            changes: vec![(0, 0)],
            ..Fake::default()
        };
        let snapshot = copy_with_backend(options(), &mut fake).unwrap();
        assert!(snapshot.changed);
        assert_eq!(snapshot.clipboard.sequence_number, 0);
        assert!(fake.waits.is_empty());
    }

    #[test]
    fn settle_is_separate_from_sequence_budget_and_remains_guarded_in_slices() {
        let mut fake = Fake {
            changes: vec![(10, 10), (15, 11)],
            ..Fake::default()
        };
        let snapshot = copy_with_backend(
            SelectionCopyOptions {
                settle_ms: 7,
                ..options()
            },
            &mut fake,
        )
        .unwrap();
        assert_eq!(fake.waits, [4, 4, 2, 4, 3]);
        assert_eq!(fake.now, 17);
        assert_eq!(snapshot.clipboard.sequence_number, 11);
        for (position, call) in fake.calls.iter().enumerate() {
            if *call == "wait" {
                assert_eq!(fake.calls[position - 2..position], ["check", "clock"]);
                assert_eq!(fake.calls[position + 1..position + 3], ["check", "clock"]);
            }
            if matches!(*call, "sequence" | "read") {
                assert_eq!(fake.calls[position - 1], "check");
                assert_eq!(fake.calls[position + 1], "check");
            }
        }
    }

    #[test]
    fn callback_and_target_failures_preserve_codes_at_every_guard_boundary() {
        for code in ["operation_guard_failed", "target_changed"] {
            let mut initial = Fake {
                initial_guard_failure: true,
                guard_code: code,
                ..Fake::default()
            };
            assert_error(
                copy_with_backend(options(), &mut initial),
                code,
                "not_started",
            );
            assert_eq!(initial.calls, ["check"]);
            for after in [
                Operation::Copy,
                Operation::Sequence,
                Operation::Wait,
                Operation::Read,
            ] {
                let mut fake = Fake {
                    fail_after: Some(after),
                    guard_code: code,
                    ..Fake::default()
                };
                if after != Operation::Wait {
                    fake.changes.push((0, 10));
                }
                assert_error(copy_with_backend(options(), &mut fake), code, "unknown");
                assert_eq!(fake.calls.iter().filter(|call| **call == "copy").count(), 1);
                if after != Operation::Read {
                    assert!(!fake.calls.contains(&"read"));
                }
            }
        }
    }

    #[test]
    fn cancellation_during_settle_stops_before_reader_without_resending() {
        let mut fake = Fake {
            fail_after: Some(Operation::Wait),
            ..Fake::changed_immediately()
        };
        assert_error(
            copy_with_backend(
                SelectionCopyOptions {
                    settle_ms: 30,
                    ..options()
                },
                &mut fake,
            ),
            "operation_guard_failed",
            "unknown",
        );
        assert_eq!(fake.waits, [4]);
        assert!(!fake.calls.contains(&"read"));
        assert_eq!(fake.calls.iter().filter(|call| **call == "copy").count(), 1);
    }

    #[test]
    fn injection_rejections_keep_precise_effect_and_never_wait_or_read() {
        for (code, effect) in [
            ("selection_busy", "not_started"),
            ("selection_input_failed", "not_started"),
            ("selection_input_failed", "unknown"),
        ] {
            let mut fake = Fake {
                copy_error: Some(SelectionDataError::new(code, "固定注入测试错误", effect)),
                ..Fake::default()
            };
            assert_error(copy_with_backend(options(), &mut fake), code, effect);
            assert!(fake.waits.is_empty());
            assert!(!fake.calls.contains(&"sequence"));
            assert!(!fake.calls.contains(&"read"));
        }
    }

    #[test]
    fn reader_error_retains_typed_code_and_post_copy_effect_without_payload_echo() {
        let mut fake = Fake {
            read_error: Some(ClipboardDataError {
                code: "unsupported_clipboard_format",
                message: "不支持的剪贴板格式。".into(),
            }),
            ..Fake::changed_immediately()
        };
        assert_error(
            copy_with_backend(options(), &mut fake),
            "unsupported_clipboard_format",
            "unknown",
        );
        let read_position = fake.calls.iter().position(|call| *call == "read").unwrap();
        assert_eq!(fake.calls[read_position + 1], "check");
    }

    #[test]
    fn clock_failure_before_copy_is_not_started_and_no_progress_or_reversal_is_bounded() {
        let mut initial = Fake {
            clock_error: true,
            ..Fake::default()
        };
        assert_error(
            copy_with_backend(options(), &mut initial),
            "selection_clock_invalid",
            "not_started",
        );
        assert!(!initial.calls.contains(&"copy"));
        for shift in [-4, -5] {
            let mut fake = Fake {
                now: 10,
                wait_shift: shift,
                ..Fake::default()
            };
            assert_error(
                copy_with_backend(options(), &mut fake),
                "selection_clock_invalid",
                "unknown",
            );
            assert_eq!(fake.waits, [4]);
            assert!(!fake.calls.contains(&"read"));
        }
    }

    #[test]
    fn sequence_and_settle_deadline_overflow_are_explicit_failures() {
        let mut waiting = Fake {
            now: u64::MAX - 2,
            ..Fake::default()
        };
        assert_error(
            copy_with_backend(options(), &mut waiting),
            "selection_clock_invalid",
            "unknown",
        );
        assert!(waiting.waits.is_empty());
        let mut settling = Fake {
            now: u64::MAX - 10,
            ..Fake::changed_immediately()
        };
        assert_error(
            copy_with_backend(
                SelectionCopyOptions {
                    settle_ms: 1000,
                    ..options()
                },
                &mut settling,
            ),
            "selection_clock_invalid",
            "unknown",
        );
        assert!(settling.waits.is_empty());
    }
}
