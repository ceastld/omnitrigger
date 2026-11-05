#[test]
fn strict_one_point_deadband_skips_adjacent_levels_in_both_directions() {
    use WmiBrightnessStepDirection::*;
    use WmiBrightnessStepSelection::*;
    let levels = [0, 38, 39, 40, 41, 42, 100];
    assert_eq!(
        plan_wmi_brightness_step(40, &levels, Increase).unwrap(),
        Set { requested: 42 }
    );
    assert_eq!(
        plan_wmi_brightness_step(40, &levels, Decrease).unwrap(),
        Set { requested: 38 }
    );
    assert_eq!(
        plan_wmi_brightness_step(40, &[39, 40, 41], Increase).unwrap(),
        NoEligibleLevel
    );
    assert_eq!(
        plan_wmi_brightness_step(40, &[39, 40, 41], Decrease).unwrap(),
        NoEligibleLevel
    );
}
#[test]
fn unsorted_duplicate_levels_choose_nearest_eligible_without_mutating_snapshot() {
    let levels = vec![100, 45, 0, 35, 60, 35, 45, 20];
    let before = levels.clone();
    assert_eq!(
        plan_wmi_brightness_step(40, &levels, WmiBrightnessStepDirection::Increase).unwrap(),
        WmiBrightnessStepSelection::Set { requested: 45 }
    );
    assert_eq!(
        plan_wmi_brightness_step(40, &levels, WmiBrightnessStepDirection::Decrease).unwrap(),
        WmiBrightnessStepSelection::Set { requested: 35 }
    );
    assert_eq!(levels, before);
}
#[test]
fn zero_candidate_and_missing_candidate_never_reach_native_writer() {
    use WmiBrightnessStepDirection::*;
    use WmiBrightnessStepSelection::*;
    for (current, levels, direction, expected) in [
        (2, vec![0], Decrease, SkipZero),
        (1, vec![0], Decrease, NoEligibleLevel),
        (0, vec![0, 1], Decrease, NoEligibleLevel),
        (100, vec![0, 100], Increase, NoEligibleLevel),
    ] {
        let selection = plan_wmi_brightness_step(current, &levels, direction).unwrap();
        assert_eq!(selection, expected);
        let result =
            submit_selected_wmi_step::<()>(selection, |_| panic!("零档或无档时禁止调用写接口"));
        assert_eq!(result.unwrap(), None);
    }
}
#[test]
fn invalid_current_or_levels_reject_before_any_submission_is_possible() {
    for (current, levels) in [
        (101, vec![0, 100]),
        (40, vec![]),
        (40, vec![101]),
        (40, vec![40; 257]),
    ] {
        assert!(
            plan_wmi_brightness_step(current, &levels, WmiBrightnessStepDirection::Increase)
                .is_err()
        );
    }
    assert!(
        plan_wmi_brightness_step(40, &vec![45; 256], WmiBrightnessStepDirection::Increase).is_ok()
    );
}
#[test]
fn array_bounds_are_bounded_without_assuming_zero_lower_bound_or_overflowing_i32() {
    assert_eq!(wmi_level_array_count(1, -10, 10).unwrap(), 21);
    assert_eq!(wmi_level_array_count(1, i32::MAX, i32::MAX).unwrap(), 1);
    assert_eq!(
        wmi_level_array_count(1, i32::MIN, i32::MIN + 255).unwrap(),
        256
    );
    for (dimensions, lower, upper) in [
        (0, 0, 1),
        (2, 0, 1),
        (1, 1, 0),
        (1, 0, 256),
        (1, i32::MIN, i32::MAX),
    ] {
        assert!(wmi_level_array_count(dimensions, lower, upper).is_err());
    }
}
#[test]
fn selected_step_submits_exact_value_once_and_preserves_failure_without_retry() {
    let selection = plan_wmi_brightness_step(
        40,
        &[0, 20, 40, 60, 100],
        WmiBrightnessStepDirection::Increase,
    )
    .unwrap();
    let mut calls = vec![];
    let result = submit_selected_wmi_step(selection, |requested| {
        calls.push(requested);
        Ok(requested)
    })
    .unwrap();
    assert_eq!(calls, vec![60]);
    assert_eq!(result, Some(60));
    let error = failure(
        SystemAdjustmentErrorCode::ConfirmationUnknown,
        "合成提交后未知",
    );
    let mut attempts = 0;
    let result = submit_selected_wmi_step::<()>(selection, |requested| {
        attempts += 1;
        assert_eq!(requested, 60);
        Err(error.clone())
    });
    assert_eq!(result.unwrap_err(), error);
    assert_eq!(attempts, 1);
}
#[test]
fn every_percent_and_direction_preserves_device_membership_and_strict_threshold() {
    let levels: Vec<u8> = (0..=100).collect();
    for current in 0..=100 {
        for direction in [
            WmiBrightnessStepDirection::Increase,
            WmiBrightnessStepDirection::Decrease,
        ] {
            match plan_wmi_brightness_step(current, &levels, direction).unwrap() {
                WmiBrightnessStepSelection::Set { requested } => {
                    assert!(requested > 0 && levels.contains(&requested));
                    match direction {
                        WmiBrightnessStepDirection::Increase => {
                            assert!(i16::from(requested) > i16::from(current) + 1)
                        }
                        WmiBrightnessStepDirection::Decrease => {
                            assert!(i16::from(requested) < i16::from(current) - 1)
                        }
                    }
                }
                WmiBrightnessStepSelection::SkipZero => assert_eq!(
                    (current, direction),
                    (2, WmiBrightnessStepDirection::Decrease)
                ),
                WmiBrightnessStepSelection::NoEligibleLevel => assert!(match direction {
                    WmiBrightnessStepDirection::Increase => current >= 99,
                    WmiBrightnessStepDirection::Decrease => current <= 1,
                }),
            }
        }
    }
}
