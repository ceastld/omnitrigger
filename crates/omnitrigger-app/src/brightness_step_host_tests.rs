fn synthetic_error() -> SystemAdjustmentError {
    SystemAdjustmentError {
        code: SystemAdjustmentErrorCode::NativeFailure,
        message: "PRIVATE_DEVICE_ID_AND_EXCEPTION".into(),
    }
}
#[test]
fn strict_request_rejects_before_pointer_or_device_access() {
    for params in [
        json!(null),
        json!([]),
        json!(["up", "physical"]),
        json!({}),
        json!({"direction":"up"}),
        json!({"direction":"UP","strategy":"physical"}),
        json!({"direction":"up","strategy":null}),
        json!({"direction":"up","strategy":"physical","retry":true}),
    ] {
        let error = execute_with(
            params,
            &CancellationToken::new(),
            None,
            || panic!("不得捕获指针"),
            |_, _, _, _| panic!("不得读写设备"),
        )
        .unwrap_err();
        assert_eq!(error.code, "invalid_input");
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    }
}
#[test]
fn cancellation_or_deadline_before_capture_is_not_started() {
    let token = CancellationToken::new();
    token.cancel();
    for (cancel, deadline, code) in [
        (&token, None, "cancelled"),
        (&CancellationToken::new(), Some(Instant::now()), "timeout"),
    ] {
        let error = execute_with(
            json!({"direction":"up","strategy":"physical"}),
            cancel,
            deadline,
            || panic!("不得捕获"),
            |_, _, _, _| panic!("不得执行"),
        )
        .unwrap_err();
        assert_eq!(error.code, code);
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    }
}
#[test]
fn only_pointer_strategies_capture_once_and_wmi_first_has_no_pointer_dependency() {
    for strategy in ["physical", "wmi_first", "physical_then_wmi"] {
        let captures = Cell::new(0);
        let invokes = Cell::new(0);
        let value = execute_with(
            json!({"direction":"down","strategy":strategy}),
            &CancellationToken::new(),
            None,
            || {
                assert_ne!(strategy, "wmi_first", "首WMI策略不得读取指针");
                captures.set(captures.get() + 1);
                Ok((7, 8))
            },
            |direction, selected, point, check| {
                invokes.set(invokes.get() + 1);
                assert_eq!(direction, BrightnessStepDirection::Decrease);
                assert_eq!(selected, strategy);
                assert_eq!(
                    point,
                    if strategy == "wmi_first" {
                        None
                    } else {
                        Some((7, 8))
                    }
                );
                check(false).unwrap();
                if strategy == "wmi_first" {
                    (None, Some(Err(synthetic_error())))
                } else {
                    (Some(Err(synthetic_error())), None)
                }
            },
        )
        .unwrap();
        assert_eq!(captures.get(), usize::from(strategy != "wmi_first"));
        assert_eq!(invokes.get(), 1);
        assert_eq!(value["strategy"], strategy);
        assert!(!value.to_string().contains("PRIVATE_DEVICE"));
    }
}
#[test]
fn cancellation_after_write_is_unknown_and_before_write_is_not_started() {
    for written in [false, true] {
        let token = CancellationToken::new();
        let error = execute_with(
            json!({"direction":"up","strategy":"physical"}),
            &token,
            None,
            || Ok((1, 2)),
            |_, _, _, check| {
                if written {
                    check(true).unwrap();
                }
                token.cancel();
                assert!(check(false).is_err());
                (Some(Err(synthetic_error())), None)
            },
        )
        .unwrap_err();
        assert_eq!(error.code, "cancelled");
        assert_eq!(
            error.effect_state.as_deref(),
            Some(if written { "unknown" } else { "not_started" })
        );
    }
}
#[test]
fn elapsed_total_deadline_after_write_is_unknown_without_a_retry() {
    let mut calls = 0;
    let start = Instant::now();
    let deadline = start + std::time::Duration::from_millis(50);
    let now = Cell::new(start);
    let error = execute_with_clock(
        json!({"direction":"up","strategy":"physical"}),
        &CancellationToken::new(),
        Some(deadline),
        || Ok((1, 2)),
        |_, _, _, check| {
            calls += 1;
            check(true).unwrap();
            now.set(deadline);
            let _ = check(false);
            (Some(Err(synthetic_error())), None)
        },
        || now.get(),
    )
    .unwrap_err();
    assert_eq!(calls, 1);
    assert_eq!(error.code, "timeout");
    assert_eq!(error.effect_state.as_deref(), Some("unknown"));
}
#[test]
fn anonymous_per_device_partial_result_keeps_unknown_and_never_serializes_labels() {
    use omnitrigger_windows::{PhysicalBrightnessMemberResult, PhysicalBrightnessReading};
    let before = PhysicalBrightnessReading {
        minimum: 0,
        current: 40,
        maximum: 100,
    };
    let native = PhysicalBrightnessSetResult {
        point: (999, 888),
        logical_device: "PRIVATE_LOGICAL".into(),
        members: vec![
            PhysicalBrightnessMemberResult {
                index: 0,
                label: "PRIVATE_DEVICE".into(),
                status: PhysicalBrightnessStatus::SetUnconfirmed {
                    before,
                    requested: 45,
                    error: synthetic_error(),
                },
            },
            PhysicalBrightnessMemberResult {
                index: 1,
                label: "PRIVATE_OTHER".into(),
                status: PhysicalBrightnessStatus::Unchanged { reading: before },
            },
        ],
    };
    let value = physical(Ok(native));
    assert_eq!(value["members"][0]["status"], "set_unconfirmed");
    assert_eq!(value["members"][1]["status"], "unchanged");
    assert!(!value.to_string().contains("PRIVATE"));
    assert!(!value.to_string().contains("999"));
}

#[test]
fn cancelled_and_timed_out_write_preserve_anonymous_completed_member_receipt() {
    use omnitrigger_windows::{PhysicalBrightnessMemberResult, PhysicalBrightnessReading};
    for timeout in [false, true] {
        let token = CancellationToken::new();
        let start = Instant::now();
        let deadline = start + std::time::Duration::from_millis(50);
        let now = Cell::new(start);
        let mut calls = 0;
        let error = execute_with_clock(
            json!({"direction":"up","strategy":"physical_then_wmi"}),
            &token,
            Some(deadline),
            || Ok((999, 888)),
            |_, _, _, check| {
                calls += 1;
                check(true).unwrap();
                if timeout {
                    now.set(deadline);
                } else {
                    token.cancel();
                }
                let before = PhysicalBrightnessReading {
                    minimum: 0,
                    current: 40,
                    maximum: 100,
                };
                let observed = PhysicalBrightnessReading {
                    current: 45,
                    ..before
                };
                (
                    Some(Ok(PhysicalBrightnessSetResult {
                        point: (999, 888),
                        logical_device: "PRIVATE_LOGICAL".into(),
                        members: vec![
                            PhysicalBrightnessMemberResult {
                                index: 0,
                                label: "PRIVATE_A".into(),
                                status: PhysicalBrightnessStatus::Confirmed {
                                    before,
                                    requested: 45,
                                    observed,
                                },
                            },
                            PhysicalBrightnessMemberResult {
                                index: 1,
                                label: "PRIVATE_B".into(),
                                status: PhysicalBrightnessStatus::ReadRejected {
                                    error: synthetic_error(),
                                },
                            },
                        ],
                    })),
                    None,
                )
            },
            || now.get(),
        )
        .unwrap_err();
        assert_eq!(calls, 1);
        assert_eq!(error.code, if timeout { "timeout" } else { "cancelled" });
        assert_eq!(error.effect_state.as_deref(), Some("unknown"));
        let receipt = error.brightness_receipt.unwrap();
        assert_eq!(receipt["physical"]["members"][0]["status"], "confirmed");
        assert_eq!(receipt["physical"]["members"][1]["status"], "read_rejected");
        assert!(receipt["wmi"].is_null());
        let text = receipt.to_string();
        assert!(!text.contains("PRIVATE"));
        assert!(!text.contains("999"));
    }
}
