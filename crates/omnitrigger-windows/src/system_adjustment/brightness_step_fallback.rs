//! 同步组合既有 DDC/WMI API；没有执行器、重试或隐式后台任务。
use super::{FirstWmiBrightnessResult, PhysicalBrightnessSetResult, SystemAdjustmentError};
#[cfg(any(windows, test))]
use super::{PhysicalBrightnessStatus, SystemAdjustmentErrorCode};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrightnessStepOutcome {
    Physical(Result<PhysicalBrightnessSetResult, SystemAdjustmentError>),
    WmiFallback {
        physical: Result<PhysicalBrightnessSetResult, SystemAdjustmentError>,
        wmi: Result<FirstWmiBrightnessResult, SystemAdjustmentError>,
    },
}

#[cfg(any(windows, test))]
pub(super) fn run_brightness_step(
    capture: impl FnOnce() -> Result<(i32, i32), SystemAdjustmentError>,
    physical: impl FnOnce((i32, i32)) -> Result<PhysicalBrightnessSetResult, SystemAdjustmentError>,
    wmi: impl FnOnce() -> Result<FirstWmiBrightnessResult, SystemAdjustmentError>,
) -> Result<BrightnessStepOutcome, SystemAdjustmentError> {
    let point = capture()?;
    let physical = physical(point);
    // Err 的允许值限定为集合原生入口写入前的选择/枚举失败。
    // NativeFailure/ConfirmationUnknown 不被推定为未写入。
    let fallback = match &physical {
        Err(error) => matches!(
            error.code,
            SystemAdjustmentErrorCode::MonitorUnavailable
                | SystemAdjustmentErrorCode::UnsupportedBrightness
        ),
        Ok(report) => {
            !report.members.is_empty()
                && report.members.iter().all(|member| {
                    matches!(member.status, PhysicalBrightnessStatus::ReadRejected { .. })
                })
        }
    };
    Ok(if fallback {
        BrightnessStepOutcome::WmiFallback {
            physical,
            wmi: wmi(),
        }
    } else {
        BrightnessStepOutcome::Physical(physical)
    })
}

#[cfg(test)]
mod tests {
    use super::super::{
        failure, AdjustmentKind, AdjustmentResult, BrightnessMethod,
        PhysicalBrightnessMemberResult, PhysicalBrightnessReading, WmiBrightnessStepSelection,
        WmiSupportedBrightnessResult,
    };
    use super::*;
    fn error(code: SystemAdjustmentErrorCode) -> SystemAdjustmentError {
        failure(code, "合成错误")
    }
    fn reading() -> PhysicalBrightnessReading {
        PhysicalBrightnessReading {
            minimum: 0,
            current: 40,
            maximum: 100,
        }
    }
    fn report(statuses: Vec<PhysicalBrightnessStatus>) -> PhysicalBrightnessSetResult {
        PhysicalBrightnessSetResult {
            point: (7, 8),
            logical_device: "合成逻辑屏幕".into(),
            members: statuses
                .into_iter()
                .enumerate()
                .map(|(index, status)| PhysicalBrightnessMemberResult {
                    index,
                    label: format!("合成物理屏幕{index}"),
                    status,
                })
                .collect(),
        }
    }
    fn wmi_result() -> FirstWmiBrightnessResult {
        FirstWmiBrightnessResult {
            instance: "合成首实例".into(),
            step: WmiSupportedBrightnessResult {
                selection: WmiBrightnessStepSelection::SkipZero,
                adjustment: AdjustmentResult {
                    kind: AdjustmentKind::Brightness,
                    previous_percent: 2,
                    requested_percent: 2,
                    current_percent: 2,
                    target_label: "合成首实例".into(),
                    monitor_device: Some("合成首实例".into()),
                    brightness_method: Some(BrightnessMethod::Wmi),
                },
            },
        }
    }
    #[test]
    fn cursor_capture_failure_stops_before_ddc_and_wmi() {
        let expected = error(SystemAdjustmentErrorCode::MonitorUnavailable);
        let result = run_brightness_step(
            || Err(expected.clone()),
            |_| panic!("没有坐标不得访问DDC"),
            || panic!("不得猜测WMI回退"),
        );
        assert_eq!(result.unwrap_err(), expected);
    }
    #[test]
    fn cursor_is_captured_once_at_execution_and_not_recaptured_on_wmi_fallback() {
        use std::cell::Cell;
        let captures = Cell::new(0);
        let pointer = Cell::new((7, 8));
        let attempts = Cell::new(0);
        let result = run_brightness_step(
            || {
                captures.set(captures.get() + 1);
                Ok(pointer.get())
            },
            |point| {
                assert_eq!(point, (7, 8));
                pointer.set((90, 100));
                Err(error(SystemAdjustmentErrorCode::UnsupportedBrightness))
            },
            || {
                attempts.set(attempts.get() + 1);
                Ok(wmi_result())
            },
        )
        .unwrap();
        assert_eq!(captures.get(), 1);
        assert_eq!(attempts.get(), 1);
        assert!(matches!(result, BrightnessStepOutcome::WmiFallback { .. }));
    }
    #[test]
    fn any_valid_read_or_write_attempt_blocks_cross_device_fallback_including_partial_failure() {
        let before = reading();
        for status in [
            PhysicalBrightnessStatus::Unchanged { reading: before },
            PhysicalBrightnessStatus::SetUnconfirmed {
                before,
                requested: 45,
                error: error(SystemAdjustmentErrorCode::ConfirmationUnknown),
            },
            PhysicalBrightnessStatus::ReadbackUnknown {
                before,
                requested: 45,
                error: error(SystemAdjustmentErrorCode::ConfirmationUnknown),
            },
            PhysicalBrightnessStatus::ReadbackDifferent {
                before,
                requested: 45,
                observed: before,
            },
            PhysicalBrightnessStatus::Confirmed {
                before,
                requested: 45,
                observed: PhysicalBrightnessReading {
                    current: 45,
                    ..before
                },
            },
        ] {
            let physical = report(vec![
                PhysicalBrightnessStatus::ReadRejected {
                    error: error(SystemAdjustmentErrorCode::NativeFailure),
                },
                status,
            ]);
            let expected = physical.clone();
            let result = run_brightness_step(
                || Ok((7, 8)),
                |_| Ok(physical),
                || panic!("有有效读取或写入尝试后不能修改另一实例"),
            )
            .unwrap();
            assert_eq!(result, BrightnessStepOutcome::Physical(Ok(expected)));
        }
    }
    #[test]
    fn every_ddc_member_read_rejected_allows_one_explicit_first_instance_step_and_preserves_both_receipts(
    ) {
        let physical = report(vec![
            PhysicalBrightnessStatus::ReadRejected {
                error: error(SystemAdjustmentErrorCode::NativeFailure),
            },
            PhysicalBrightnessStatus::ReadRejected {
                error: error(SystemAdjustmentErrorCode::UnsupportedBrightness),
            },
        ]);
        let original = physical.clone();
        let expected = wmi_result();
        let mut attempts = 0;
        let result = run_brightness_step(
            || Ok((7, 8)),
            |_| Ok(physical),
            || {
                attempts += 1;
                Ok(expected.clone())
            },
        )
        .unwrap();
        assert_eq!(attempts, 1);
        assert_eq!(
            result,
            BrightnessStepOutcome::WmiFallback {
                physical: Ok(original),
                wmi: Ok(expected)
            }
        );
    }
    #[test]
    fn allowed_prewrite_enumeration_failure_and_wmi_error_are_preserved_without_retry() {
        for code in [
            SystemAdjustmentErrorCode::UnsupportedBrightness,
            SystemAdjustmentErrorCode::MonitorUnavailable,
        ] {
            let ddc_error = error(code);
            let wmi_error = error(SystemAdjustmentErrorCode::ConfirmationUnknown);
            let mut attempts = 0;
            let result = run_brightness_step(
                || Ok((7, 8)),
                |_| Err(ddc_error.clone()),
                || {
                    attempts += 1;
                    Err(wmi_error.clone())
                },
            )
            .unwrap();
            assert_eq!(attempts, 1);
            assert_eq!(
                result,
                BrightnessStepOutcome::WmiFallback {
                    physical: Err(ddc_error),
                    wmi: Err(wmi_error)
                }
            );
        }
    }
    #[test]
    fn unknown_effect_errors_or_invalid_empty_report_never_become_a_fallback_permission() {
        for code in [
            SystemAdjustmentErrorCode::NativeFailure,
            SystemAdjustmentErrorCode::ConfirmationUnknown,
            SystemAdjustmentErrorCode::AmbiguousMonitor,
            SystemAdjustmentErrorCode::InvalidDelta,
        ] {
            let expected = error(code);
            assert_eq!(
                run_brightness_step(
                    || Ok((7, 8)),
                    |_| Err(expected.clone()),
                    || panic!("未知影响不得回退")
                )
                .unwrap(),
                BrightnessStepOutcome::Physical(Err(expected))
            );
        }
        assert!(matches!(
            run_brightness_step(
                || Ok((7, 8)),
                |_| Ok(report(vec![])),
                || panic!("空回执不得回退")
            )
            .unwrap(),
            BrightnessStepOutcome::Physical(Ok(_))
        ));
    }
}
