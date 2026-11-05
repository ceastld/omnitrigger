use super::*;

fn args(arguments: &[&str]) -> Vec<String> {
    arguments
        .iter()
        .map(|argument| (*argument).to_string())
        .collect()
}

fn rejected(arguments: &[&str]) {
    let error = parse(&args(arguments)).unwrap_err();
    assert_eq!(error.code, "invalid_cli_arguments");
    assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    let response = serde_json::json!({"status":"rejected","error":error});
    assert_eq!(response["status"], "rejected");
    assert_eq!(response["error"]["effect_state"], "not_started");
}

#[test]
fn missing_policy_preserves_default_and_does_not_revalidate_legacy_arguments() {
    for input in [
        vec![],
        vec!["app.exe"],
        vec!["--config"],
        vec!["app.exe", "--wheel"],
        vec!["app.exe", "--legacy-unknown-option"],
        vec!["app.exe", "--check-config"],
        vec!["app.exe", "--run-action", "call.json"],
        vec!["app.exe", "--target-context"],
        vec!["app.exe", "--browser-connector", "status"],
        vec!["app.exe", "--browser-action", "call.json"],
        vec!["app.exe", "--create-script-action", "new-project"],
        vec!["app.exe", "--install-script-runtime", "runtime.zip"],
        vec!["app.exe", "--notification-smoke", "owned-output"],
        vec!["app.exe", "--preview-system-hud", "brightness"],
        vec!["app.exe", "--adjust-volume", "5"],
        vec!["app.exe", "--check-system-controls"],
        vec!["app.exe", "--smoke-test"],
        vec!["app.exe", "--settings"],
        vec!["app.exe", "--wheel-editor"],
    ] {
        let arguments = args(&input);
        let original = arguments.clone();
        assert_eq!(parse(&arguments).unwrap(), WheelInputPolicy::Unfiltered);
        assert_eq!(arguments, original, "不移除或重写旧 CLI 参数");
    }
}

#[test]
fn explicit_policy_is_typed_and_accepts_both_argv_forms() {
    assert_eq!(
        parse(&args(&["app.exe", FLAG, "unfiltered"])).unwrap(),
        WheelInputPolicy::Unfiltered
    );
    assert_eq!(
        parse(&args(&[FLAG, "small-vertical-150ms"])).unwrap(),
        WheelInputPolicy::SmallVertical150ms
    );
    assert_eq!(
        parse(&args(&[
            "D:/has space/应用.exe",
            FLAG,
            "small-vertical-150ms"
        ]))
        .unwrap(),
        WheelInputPolicy::SmallVertical150ms
    );
}

#[test]
fn explicit_policy_accepts_every_existing_hook_gui_entrypoint_in_either_order() {
    for option in [
        vec!["--ui-smoke"],
        vec!["--settings"],
        vec!["--browser-connector-settings"],
        vec!["--panel"],
        vec!["--wheel", "wheel-id"],
        vec!["--wheel-editor"],
        vec!["--config", "D:/合成测试/config with space.json"],
        vec!["--wheel", "wheel-id", "--wheel-editor"],
    ] {
        for policy in ["unfiltered", "small-vertical-150ms"] {
            let expected = if policy == "unfiltered" {
                WheelInputPolicy::Unfiltered
            } else {
                WheelInputPolicy::SmallVertical150ms
            };
            let mut first = vec!["app.exe", FLAG, policy];
            first.extend(option.iter().copied());
            assert_eq!(parse(&args(&first)).unwrap(), expected);
            let mut last = vec!["app.exe"];
            last.extend(option.iter().copied());
            last.extend([FLAG, policy]);
            assert_eq!(parse(&args(&last)).unwrap(), expected);
        }
    }
}

#[test]
fn smoke_test_accepts_the_actual_typed_policy_with_optional_config() {
    for (name, expected) in [
        ("unfiltered", WheelInputPolicy::Unfiltered),
        ("small-vertical-150ms", WheelInputPolicy::SmallVertical150ms),
    ] {
        for arguments in [
            vec!["app.exe", "--smoke-test", FLAG, name],
            vec!["app.exe", FLAG, name, "--smoke-test"],
            vec![
                "app.exe",
                "--config",
                "owned.json",
                FLAG,
                name,
                "--smoke-test",
            ],
        ] {
            assert_eq!(parse(&args(&arguments)).unwrap(), expected);
        }
    }
}

#[test]
fn invalid_duplicate_missing_and_equals_policy_values_are_structured_rejections() {
    for arguments in [
        vec!["app.exe", FLAG],
        vec!["app.exe", FLAG, ""],
        vec!["app.exe", FLAG, "--smoke-test"],
        vec!["app.exe", FLAG, "unknown"],
        vec!["app.exe", FLAG, "Unfiltered"],
        vec!["app.exe", FLAG, "small_vertical_150ms"],
        vec!["app.exe", FLAG, " unfiltered "],
        vec!["app.exe", "--wheel-input-policy=unfiltered"],
        vec!["app.exe", "--wheel-input-policy="],
        vec!["app.exe", FLAG, "unfiltered", FLAG, "unfiltered"],
        vec![
            "app.exe",
            FLAG,
            "unfiltered",
            "--settings",
            FLAG,
            "small-vertical-150ms",
        ],
        vec![
            "app.exe",
            FLAG,
            "unfiltered",
            "--wheel-input-policy=small-vertical-150ms",
        ],
    ] {
        rejected(&arguments);
    }
}

#[test]
fn explicit_policy_rejects_all_no_hook_operations_before_dispatch() {
    for operation in [
        vec!["--check-config"],
        vec!["--check-action", "call.json"],
        vec!["--run-action", "call.json"],
        vec!["--list-package-actions"],
        vec!["--create-script-action", "new-project"],
        vec![
            "--install-script-runtime",
            "runtime.zip",
            "--sha256",
            "synthetic-hash",
        ],
        vec!["--target-context", "target.json"],
        vec!["--stream-action-events"],
        vec!["--sha256", "synthetic-hash"],
        vec!["--browser-action", "call.json"],
        vec!["--browser-connector", "status"],
        vec!["--browser-connector", "install"],
        vec!["--browser-connector", "register", "synthetic-id"],
        vec!["--browser-connector", "start"],
        vec!["--browser-connector", "stop"],
        vec!["--browser-connector", "refresh"],
        vec!["--browser-connector", "uninstall"],
        vec!["--check-system-controls"],
        vec!["--adjust-volume", "5"],
        vec!["--adjust-brightness", "5"],
        vec!["--preview-system-hud", "brightness"],
        vec!["--notification-smoke", "owned-output"],
    ] {
        for policy in ["unfiltered", "small-vertical-150ms"] {
            let mut first = vec!["app.exe", FLAG, policy];
            first.extend(operation.iter().copied());
            rejected(&first);
            let mut last = vec!["app.exe"];
            last.extend(operation.iter().copied());
            last.extend([FLAG, policy]);
            rejected(&last);
        }
    }
}

#[test]
fn smoke_test_does_not_hide_an_incompatible_operation_or_debug_shortcut() {
    for operation in [
        "--check-config",
        "--run-action",
        "--browser-connector",
        "--notification-smoke",
        "--preview-system-hud",
        "--adjust-volume",
        "--create-script-action",
        "--install-script-runtime",
    ] {
        rejected(&[
            "app.exe",
            "--smoke-test",
            FLAG,
            "small-vertical-150ms",
            operation,
            "synthetic-value",
        ]);
        rejected(&[
            "app.exe",
            operation,
            "synthetic-value",
            "--config",
            "not-read.json",
            FLAG,
            "unfiltered",
            "--smoke-test",
        ]);
    }
}

#[test]
fn explicit_policy_rejects_missing_gui_values_unknown_arguments_and_duplicate_flags() {
    for tail in [
        vec!["--config"],
        vec!["--wheel"],
        vec!["--config", ""],
        vec!["--wheel", ""],
        vec!["--config", "--settings"],
        vec!["--wheel", "--wheel-editor"],
        vec!["--config", "owned.json", "--config", "other.json"],
        vec!["--panel", "--panel"],
        vec!["--smoke-test", "--smoke-test"],
        vec!["--unknown"],
        vec!["extra-positional"],
        vec!["--point", "0,0"],
    ] {
        let mut arguments = vec!["app.exe", FLAG, "unfiltered"];
        arguments.extend(tail);
        rejected(&arguments);
    }
}

#[test]
fn rejected_arguments_are_not_echoed_or_modified() {
    let arguments = args(&[
        "app.exe",
        "--config",
        "D:/synthetic-private-path/credential-placeholder.json",
        FLAG,
        "unfiltered",
        "--run-action",
        "synthetic-action.json",
    ]);
    let original = arguments.clone();
    let error = parse(&arguments).unwrap_err();
    assert!(!error.message.contains("synthetic-private-path"));
    assert!(!error.message.contains("credential-placeholder"));
    assert!(!error.message.contains("synthetic-action"));
    assert_eq!(arguments, original);
}
