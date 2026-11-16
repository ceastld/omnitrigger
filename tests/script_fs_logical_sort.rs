use super::*;
use std::{cell::Cell, time::Duration};

fn logical_later() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

fn assert_logical_error(error: ActionError, code: &str) {
    assert_eq!(error.code, code);
    assert_eq!(error.method.as_deref(), Some("fs.sort_logical_paths"));
    assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    assert!(error.required_permissions.is_empty());
    assert!(!error.message.contains("private-input"));
    assert!(!error.message.contains("private-os"));
}

fn validate_logical(params: Value) -> Result<Value, ActionError> {
    sort_logical_paths_with(
        params,
        &CancellationToken::new(),
        logical_later(),
        Instant::now,
        |paths| Ok(paths.to_vec()),
    )
}

#[test]
fn fs_sort_logical_rejects_invalid_shape_before_backend() {
    for params in [
        json!(null),
        json!([]),
        json!([["private-input"]]),
        json!({}),
        json!({"paths":null}),
        json!({"paths":"private-input"}),
        json!({"paths":[null]}),
        json!({"paths":[42]}),
        json!({"paths":[],"extra":"private-input"}),
    ] {
        assert_logical_error(
            sort_logical_paths_with(
                params,
                &CancellationToken::new(),
                logical_later(),
                Instant::now,
                |_| panic!("无效参数不得进入系统比较器"),
            )
            .unwrap_err(),
            "invalid_input",
        );
    }
}

#[test]
fn fs_sort_logical_accepts_empty_nul_relative_duplicates_and_item_count_boundary() {
    let paths = json!([
        "",
        "relative/path",
        "a\0tail",
        "../same",
        "../same",
        " C:/keep  "
    ]);
    assert_eq!(validate_logical(json!({"paths":paths})).unwrap(), paths);
    assert_eq!(validate_logical(json!({"paths":[]})).unwrap(), json!([]));
    let paths = vec![String::new(); 1024];
    assert_eq!(
        validate_logical(json!({"paths":paths})).unwrap(),
        json!(paths)
    );
    assert_logical_error(
        validate_logical(json!({"paths":vec![String::new();1025]})).unwrap_err(),
        "invalid_input",
    );
}

#[test]
fn fs_sort_logical_counts_utf16_units_and_exact_serialized_object_bytes() {
    let accepted = format!("{}😀", "a".repeat(32765));
    assert_eq!(accepted.encode_utf16().count(), 32767);
    assert_eq!(
        validate_logical(json!({"paths":[accepted]})).unwrap(),
        json!([accepted])
    );
    assert_logical_error(
        validate_logical(json!({"paths":[format!("{}😀","a".repeat(32766))]})).unwrap_err(),
        "invalid_input",
    );
    let overhead = serde_json::to_vec(&json!({"paths":["",""]})).unwrap().len();
    let first = "a".repeat(32767);
    let second = "b".repeat(64 * 1024 - overhead - first.len());
    let exact = json!({"paths":[first,second]});
    assert_eq!(serde_json::to_vec(&exact).unwrap().len(), 64 * 1024);
    assert_eq!(validate_logical(exact).unwrap(), json!([first, second]));
    assert_logical_error(
        validate_logical(json!({"paths":[first,format!("{second}x")]})).unwrap_err(),
        "invalid_input",
    );
    let escaped = json!({"paths":["\0".repeat(10921)]});
    assert!(serde_json::to_vec(&escaped).unwrap().len() > 64 * 1024);
    assert_logical_error(validate_logical(escaped).unwrap_err(), "invalid_input");
}

#[test]
fn fs_sort_logical_checks_cancellation_and_deadline_before_and_after_backend() {
    let base = Instant::now();
    let deadline = base + Duration::from_secs(5);
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert_logical_error(
        sort_logical_paths_with(
            json!({"paths":[]}),
            &cancelled,
            deadline,
            || base,
            |_| panic!("取消后不得排序"),
        )
        .unwrap_err(),
        "cancelled",
    );
    assert_logical_error(
        sort_logical_paths_with(
            json!({"paths":[]}),
            &CancellationToken::new(),
            deadline,
            || deadline,
            |_| panic!("截止后不得排序"),
        )
        .unwrap_err(),
        "timeout",
    );
    let token = CancellationToken::new();
    assert_logical_error(
        sort_logical_paths_with(
            json!({"paths":["a"]}),
            &token,
            deadline,
            || base,
            |paths| {
                token.cancel();
                Ok(paths.to_vec())
            },
        )
        .unwrap_err(),
        "cancelled",
    );
    let completed = Cell::new(false);
    assert_logical_error(
        sort_logical_paths_with(
            json!({"paths":["a"]}),
            &CancellationToken::new(),
            deadline,
            || if completed.get() { deadline } else { base },
            |paths| {
                completed.set(true);
                Ok(paths.to_vec())
            },
        )
        .unwrap_err(),
        "timeout",
    );
}

#[test]
fn fs_sort_logical_hides_unknown_backend_errors() {
    let error = sort_logical_paths_with(
        json!({"paths":["private-input"]}),
        &CancellationToken::new(),
        logical_later(),
        Instant::now,
        |_| {
            let mut error = ActionError::new("private-os-code", "private-os: private-input");
            error.effect_state = Some("unknown".into());
            error.required_permissions.push("private-input".into());
            Err(error)
        },
    )
    .unwrap_err();
    assert_logical_error(error, "fs_unavailable");
}

#[test]
fn fs_sort_logical_real_host_uses_no_ui_or_desktop_target() {
    use omnitrigger_actions::NativeHost;
    struct NoTarget;
    impl NativeHost for NoTarget {
        fn native_context_available(&self) -> bool {
            false
        }
        fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
            panic!("纯字符串排序不得转发到桌面或要求目标")
        }
    }
    let host = crate::script_ui::Host::new(NoTarget, None);
    let result = host.call_with_deadline(
        "fs.sort_logical_paths",
        json!({"paths":["relative20", "relative3"]}),
        &CancellationToken::new(),
        logical_later(),
    );
    #[cfg(windows)]
    assert_eq!(result.unwrap(), json!(["relative3", "relative20"]));
    #[cfg(not(windows))]
    assert_logical_error(result.unwrap_err(), "fs_unavailable");
}
