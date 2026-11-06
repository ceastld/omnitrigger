use super::*;
use std::time::Duration;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}
fn params() -> Value {
    json!({"path":"C:/synthetic/","max_entries":2})
}
fn entry(name: &str) -> LocalDirectoryEntry {
    LocalDirectoryEntry {
        name: name.into(),
        path: format!("C:/synthetic/{name}"),
        kind: "file",
        reason: None,
    }
}
fn assert_error(error: ActionError, code: &str) {
    assert_eq!(error.code, code);
    assert_eq!(error.method.as_deref(), Some("fs.read_directory"));
    assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    assert!(!error.message.contains("sensitive-marker"));
}
#[test]
fn read_directory_adapter_calls_real_boundary_with_original_path_and_limit() {
    let result = read_directory_with(
        params(),
        &CancellationToken::new(),
        deadline(),
        |path, limit, check| {
            assert_eq!(path, "C:/synthetic/");
            assert_eq!(limit, 2);
            check()?;
            Ok(vec![entry("a")])
        },
    )
    .unwrap();
    assert_eq!(
        result,
        json!({"entries":[{"name":"a","path":"C:/synthetic/a","kind":"file","reason":null}],"complete":true})
    );
}
#[test]
fn invalid_directory_wire_values_never_reach_platform() {
    for value in [
        Value::Null,
        json!([]),
        json!(["C:/synthetic/", 2]),
        json!({"path":"C:/synthetic/"}),
        json!({"path":"C:/synthetic/","max_entries":0}),
        json!({"path":"C:/synthetic/","max_entries":1025}),
        json!({"path":"C:/synthetic/","max_entries":1.5}),
        json!({"path":"C:/a/../b","max_entries":1}),
        json!({"path":"C:/synthetic/","max_entries":1,"extra":true}),
    ] {
        assert_error(
            read_directory_with(value, &CancellationToken::new(), deadline(), |_, _, _| {
                panic!("invalid input reached platform")
            })
            .unwrap_err(),
            "invalid_input",
        );
    }
    assert_error(
        read_directory_with(
            json!({"path":"C:/OneDrive/a","max_entries":1}),
            &CancellationToken::new(),
            deadline(),
            |_, _, _| panic!("cloud path reached platform"),
        )
        .unwrap_err(),
        "path_unavailable",
    );
}
#[test]
fn directory_adapter_preserves_known_codes_and_sanitizes_error_text() {
    for code in [
        "path_not_local",
        "path_unavailable",
        "path_missing",
        "not_directory",
        "access_denied",
        "directory_too_large",
        "directory_changed",
        "fs_unavailable",
        "fs_error",
    ] {
        assert_error(
            read_directory_with(
                params(),
                &CancellationToken::new(),
                deadline(),
                |_, _, _| {
                    Err(LocalStatError {
                        code,
                        message: "sensitive-marker".into(),
                    })
                },
            )
            .unwrap_err(),
            code,
        );
    }
    assert_error(
        read_directory_with(
            params(),
            &CancellationToken::new(),
            deadline(),
            |_, _, _| {
                Err(LocalStatError {
                    code: "unexpected",
                    message: "sensitive-marker".into(),
                })
            },
        )
        .unwrap_err(),
        "fs_error",
    );
}
#[test]
fn late_cancellation_wins_over_success_or_platform_error() {
    for success in [true, false] {
        let token = CancellationToken::new();
        let result = read_directory_with(params(), &token, deadline(), |_, _, _| {
            token.cancel();
            if success {
                Ok(vec![entry("a")])
            } else {
                Err(LocalStatError {
                    code: "access_denied",
                    message: String::new(),
                })
            }
        });
        assert_error(result.unwrap_err(), "cancelled");
    }
    assert_error(
        read_directory_with(
            params(),
            &CancellationToken::new(),
            Instant::now(),
            |_, _, _| panic!("expired query"),
        )
        .unwrap_err(),
        "timeout",
    );
}
#[test]
fn adapter_does_not_publish_oversized_platform_output() {
    assert_error(
        read_directory_with(
            params(),
            &CancellationToken::new(),
            deadline(),
            |_, _, _| Ok(vec![entry("a"), entry("b"), entry("c")]),
        )
        .unwrap_err(),
        "directory_too_large",
    );
    assert_error(
        read_directory_with(
            params(),
            &CancellationToken::new(),
            deadline(),
            |_, _, _| Ok(vec![entry(&"中".repeat(12000))]),
        )
        .unwrap_err(),
        "directory_too_large",
    );
}
