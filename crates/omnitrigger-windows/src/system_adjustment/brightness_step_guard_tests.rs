// 被既有 brightness_step_fallback::tests include；复用 reading/report/wmi_result。
use super::super::physical_brightness_set::{step_members_if, PhysicalBrightnessPort};
use std::cell::Cell;

#[derive(Clone, Copy, Debug)]
enum GuardStop {
    Cancelled,
    Timeout,
}
fn guard_stop_error(reason: GuardStop) -> SystemAdjustmentError {
    failure(
        SystemAdjustmentErrorCode::ConfirmationUnknown,
        match reason {
            GuardStop::Cancelled => "synthetic cancelled",
            GuardStop::Timeout => "synthetic deadline reached",
        },
    )
}

#[test]
fn guard_initial_stop_is_before_physical_wmi_or_any_write_boundary() {
    for reason in [GuardStop::Cancelled, GuardStop::Timeout] {
        let physical_calls = Cell::new(0);
        let wmi_calls = Cell::new(0);
        let writes = Cell::new(0);
        let result = run_guarded_brightness_step(
            (7, 8),
            |_| {
                physical_calls.set(physical_calls.get() + 1);
                panic!("初始已停止，不得进入物理provider")
            },
            |_| {
                wmi_calls.set(wmi_calls.get() + 1);
                panic!("初始已停止，不得进入WMI provider")
            },
            &mut |write| {
                if write {
                    writes.set(writes.get() + 1);
                }
                Err(guard_stop_error(reason))
            },
        );
        assert_eq!(result.unwrap_err(), guard_stop_error(reason));
        assert_eq!(physical_calls.get(), 0);
        assert_eq!(wmi_calls.get(), 0);
        assert_eq!(writes.get(), 0); // 写前边界；由现有AdjustmentControl映射not_started。
    }
}

fn guard_fallback_eligible(
    as_error: bool,
) -> Result<PhysicalBrightnessSetResult, SystemAdjustmentError> {
    if as_error {
        Err(error(SystemAdjustmentErrorCode::UnsupportedBrightness))
    } else {
        Ok(report(vec![PhysicalBrightnessStatus::ReadRejected {
            error: error(SystemAdjustmentErrorCode::NativeFailure),
        }]))
    }
}

#[test]
fn guard_post_physical_stop_blocks_otherwise_eligible_wmi_and_keeps_exact_physical() {
    // 必须使用原本可回退的结果，避免仅因Confirmed/未知写入本来就禁止回退而误判守卫通过。
    for reason in [GuardStop::Cancelled, GuardStop::Timeout] {
        for as_error in [false, true] {
            let stopped = Cell::new(false);
            let physical_calls = Cell::new(0);
            let wmi_calls = Cell::new(0);
            let writes = Cell::new(0);
            let expected = guard_fallback_eligible(as_error);
            let actual = run_guarded_brightness_step(
                (7, 8),
                |_| {
                    physical_calls.set(physical_calls.get() + 1);
                    stopped.set(true);
                    expected.clone()
                },
                |_| {
                    wmi_calls.set(wmi_calls.get() + 1);
                    panic!("physical完成后已取消/超时，不得进入原本可回退的WMI")
                },
                &mut |write| {
                    if stopped.get() {
                        return Err(guard_stop_error(reason));
                    }
                    if write {
                        writes.set(writes.get() + 1);
                    }
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(actual, BrightnessStepOutcome::Physical(expected));
            assert_eq!(physical_calls.get(), 1);
            assert_eq!(wmi_calls.get(), 0);
            assert_eq!(writes.get(), 0);
        }
    }
}

#[test]
fn guard_terminal_stop_keeps_already_confirmed_physical_after_allowed_write() {
    for reason in [GuardStop::Cancelled, GuardStop::Timeout] {
        let stopped = Cell::new(false);
        let writes = Cell::new(0);
        let wmi_calls = Cell::new(0);
        let before = reading();
        let expected = report(vec![PhysicalBrightnessStatus::Confirmed {
            before,
            requested: 45,
            observed: PhysicalBrightnessReading {
                current: 45,
                ..before
            },
        }]);
        let actual = run_guarded_brightness_step(
            (7, 8),
            |check| {
                check(true).unwrap();
                // provider已产生完整回执，取消/期限恰发生在末尾守卫之前。
                stopped.set(true);
                Ok(expected.clone())
            },
            |_| {
                wmi_calls.set(wmi_calls.get() + 1);
                panic!("已写后的停止不得改投WMI")
            },
            &mut |write| {
                if stopped.get() {
                    return Err(guard_stop_error(reason));
                }
                if write {
                    writes.set(writes.get() + 1);
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(actual, BrightnessStepOutcome::Physical(Ok(expected)));
        assert_eq!(writes.get(), 1); // 已许可写边界；现有AdjustmentControl必须保持unknown。
        assert_eq!(wmi_calls.get(), 0);
    }
}

#[derive(Debug, PartialEq, Eq)]
enum GuardPortEvent {
    Read(usize),
    Set(usize, u32),
}
struct GuardPort<'a> {
    stopped: &'a Cell<bool>,
    reads: [usize; 3],
    events: Vec<GuardPortEvent>,
}
impl PhysicalBrightnessPort for GuardPort<'_> {
    fn count(&self) -> usize {
        3
    }
    fn label(&self, index: usize) -> String {
        format!("合成物理屏幕{index}")
    }
    fn read(&mut self, index: usize) -> Result<PhysicalBrightnessReading, SystemAdjustmentError> {
        self.events.push(GuardPortEvent::Read(index));
        self.reads[index] += 1;
        match (index, self.reads[index]) {
            (0, 1) => Ok(reading()),
            (0, 2) => Ok(PhysicalBrightnessReading {
                current: 45,
                ..reading()
            }),
            (1, 1) => Ok(PhysicalBrightnessReading {
                current: 20,
                ..reading()
            }),
            _ => panic!("停止后不得继续原生读取或重试"),
        }
    }
    fn set(&mut self, index: usize, value: u32) -> Result<(), SystemAdjustmentError> {
        self.events.push(GuardPortEvent::Set(index, value));
        if index == 1 {
            self.stopped.set(true); // 第二台写调用返回时停止：既可能已作用，又没有读回确认。
        }
        Ok(())
    }
}

#[test]
fn guard_partial_provider_tail_and_strategy_tail_both_keep_progress_without_wmi_or_more_io() {
    for reason in [GuardStop::Cancelled, GuardStop::Timeout] {
        let stopped = Cell::new(false);
        let writes = Cell::new(0);
        let wmi_calls = Cell::new(0);
        let mut port = GuardPort {
            stopped: &stopped,
            reads: [0; 3],
            events: Vec::new(),
        };
        let actual = run_guarded_brightness_step(
            (7, 8),
            |check| {
                let members = step_members_if(
                    &mut port,
                    super::super::BrightnessStepDirection::Increase,
                    check,
                )?;
                Ok(PhysicalBrightnessSetResult {
                    point: (7, 8),
                    logical_device: "合成逻辑屏幕".into(),
                    members,
                })
            },
            |_| {
                wmi_calls.set(wmi_calls.get() + 1);
                panic!("部分写入后不得进入WMI")
            },
            &mut |write| {
                if stopped.get() {
                    return Err(guard_stop_error(reason));
                }
                if write {
                    writes.set(writes.get() + 1);
                }
                Ok(())
            },
        )
        .unwrap();
        let before = reading();
        let expected = report(vec![
            PhysicalBrightnessStatus::Confirmed {
                before,
                requested: 45,
                observed: PhysicalBrightnessReading {
                    current: 45,
                    ..before
                },
            },
            PhysicalBrightnessStatus::SetUnconfirmed {
                before: PhysicalBrightnessReading {
                    current: 20,
                    ..before
                },
                requested: 25,
                error: guard_stop_error(reason),
            },
            PhysicalBrightnessStatus::ReadRejected {
                error: guard_stop_error(reason),
            },
        ]);
        assert_eq!(actual, BrightnessStepOutcome::Physical(Ok(expected)));
        assert_eq!(writes.get(), 2);
        assert_eq!(wmi_calls.get(), 0);
        assert_eq!(
            port.events,
            vec![
                GuardPortEvent::Read(0),
                GuardPortEvent::Set(0, 45),
                GuardPortEvent::Read(0),
                GuardPortEvent::Read(1),
                GuardPortEvent::Set(1, 25)
            ]
        );
    }
}

#[test]
fn guard_live_control_still_enters_wmi_once_for_the_same_eligible_physical_results() {
    // 活守卫对照：证明以上两个eligible反例真正经过回退资格判断，而非测试永远禁止WMI。
    for as_error in [false, true] {
        let expected = guard_fallback_eligible(as_error);
        let wmi_calls = Cell::new(0);
        let actual = run_guarded_brightness_step(
            (7, 8),
            |_| expected.clone(),
            |_| {
                wmi_calls.set(wmi_calls.get() + 1);
                Ok(wmi_result())
            },
            &mut |write| {
                assert!(!write);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            actual,
            BrightnessStepOutcome::WmiFallback {
                physical: expected,
                wmi: Ok(wmi_result())
            }
        );
        assert_eq!(wmi_calls.get(), 1);
    }
}
