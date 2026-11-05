//! 仅用于显式首枚举 WMI 入口；保持第一项身份，不因检查失败选择下一项。
#[cfg(any(windows, test))]
pub(super) fn with_first_wmi_row<T, R>(
    rows: impl IntoIterator<Item = T>,
    inspect: impl FnOnce(T) -> Result<R, super::SystemAdjustmentError>,
) -> Result<R, super::SystemAdjustmentError> {
    let row = rows.into_iter().next().ok_or_else(|| {
        super::failure(
            super::SystemAdjustmentErrorCode::UnsupportedBrightness,
            "没有可枚举的 WMI 亮度实例，未提交请求。",
        )
    })?;
    inspect(row)
}

#[cfg(test)]
mod tests {
    use super::super::{failure, SystemAdjustmentErrorCode};
    use super::*;
    #[derive(Debug, PartialEq, Eq)]
    struct Snapshot {
        instance: &'static str,
        current: u8,
        levels: Vec<u8>,
    }
    fn row(instance: &'static str, current: u8) -> Snapshot {
        Snapshot {
            instance,
            current,
            levels: vec![0, 20, 40, 60, 100],
        }
    }
    #[test]
    fn first_row_and_its_original_levels_and_identity_are_preserved_without_sorting() {
        let expected = row("DISPLAY\\Z-FIRST", 40);
        let selected = with_first_wmi_row(
            vec![row("DISPLAY\\Z-FIRST", 40), row("DISPLAY\\A-SECOND", 80)],
            Ok,
        )
        .unwrap();
        assert_eq!(selected, expected);
    }
    #[test]
    fn lazy_iterator_is_never_advanced_to_second_row_and_inspector_runs_once() {
        let mut calls = 0;
        let rows = std::iter::once(row("FIRST", 40))
            .chain(std::iter::from_fn(|| panic!("禁止探测下一实例")));
        let instance = with_first_wmi_row(rows, |snapshot| {
            calls += 1;
            Ok(snapshot.instance)
        })
        .unwrap();
        assert_eq!(instance, "FIRST");
        assert_eq!(calls, 1);
    }
    #[test]
    fn empty_list_does_not_run_inspector_or_writer() {
        let result = with_first_wmi_row(Vec::<Snapshot>::new(), |_| -> Result<(), _> {
            panic!("空集合不得写入")
        });
        assert_eq!(
            result.unwrap_err().code,
            SystemAdjustmentErrorCode::UnsupportedBrightness
        );
    }
    #[test]
    fn invalid_first_or_submission_failure_is_returned_without_retarget_or_retry() {
        let error = failure(
            SystemAdjustmentErrorCode::ConfirmationUnknown,
            "首实例提交结果未知",
        );
        let mut attempts = 0;
        let result = with_first_wmi_row(vec![row("FIRST", 40), row("SECOND", 80)], |snapshot| {
            attempts += 1;
            assert_eq!(snapshot.instance, "FIRST");
            Err::<(), _>(error.clone())
        });
        assert_eq!(result.unwrap_err(), error);
        assert_eq!(attempts, 1);
    }
    #[test]
    fn first_instance_step_uses_first_levels_and_does_not_reach_second_zero_or_writer() {
        use super::super::wmi_brightness_levels::{
            plan_wmi_brightness_step, submit_selected_wmi_step, WmiBrightnessStepDirection,
            WmiBrightnessStepSelection,
        };
        let mut writes = vec![];
        let result = with_first_wmi_row(vec![row("FIRST", 40), row("SECOND", 80)], |snapshot| {
            let plan = plan_wmi_brightness_step(
                snapshot.current,
                &snapshot.levels,
                WmiBrightnessStepDirection::Increase,
            )?;
            submit_selected_wmi_step(plan, |requested| {
                writes.push((snapshot.instance, requested));
                Ok(requested)
            })
        })
        .unwrap();
        assert_eq!(result, Some(60));
        assert_eq!(writes, vec![("FIRST", 60)]);
        let result = with_first_wmi_row(
            vec![
                Snapshot {
                    instance: "FIRST",
                    current: 2,
                    levels: vec![0],
                },
                row("SECOND", 80),
            ],
            |snapshot| {
                let plan = plan_wmi_brightness_step(
                    snapshot.current,
                    &snapshot.levels,
                    WmiBrightnessStepDirection::Decrease,
                )?;
                assert_eq!(plan, WmiBrightnessStepSelection::SkipZero);
                submit_selected_wmi_step::<()>(plan, |_| panic!("零档不得转第二实例或写入"))
            },
        )
        .unwrap();
        assert_eq!(result, None);
    }
}
