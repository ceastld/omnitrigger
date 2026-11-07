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
        metadata: None,
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
            assert_eq!(limit.max_entries, 2);
            assert_eq!(limit.order, DirectoryOrder::Name);
            assert!(!limit.metadata);
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

#[test]
fn explicit_directory_options_preserve_native_projection_and_decimal_time_precision() {
    let value = read_directory_with(
        json!({"path":"C:/synthetic/","max_entries":2,"order":"native","metadata":true,
            "entry_type":"file","search_pattern":"*.xls","match_mode":"windows-find"}),
        &CancellationToken::new(),
        deadline(),
        |path, options, check| {
            assert_eq!(path, "C:/synthetic/");
            assert_eq!(options.order, DirectoryOrder::Native);
            assert_eq!(options.entry_type, DirectoryEntryType::File);
            assert!(options.metadata);
            assert_eq!(options.search_pattern.as_deref(), Some("*.xls"));
            check()?;
            let mut result = entry("a.xls");
            result.metadata = Some(omnitrigger_windows::LocalDirectoryMetadata {
                ordinal: 0,
                attributes: 6,
                creation_utc_100ns: 134_064_342_001_234_567,
                access_utc_100ns: 134_064_342_001_234_567,
                write_utc_100ns: 134_064_342_001_234_568,
                creation_local_ticks: 638_975_574_001_234_567,
                access_local_ticks: 638_975_574_001_234_567,
                write_local_ticks: 638_975_574_001_234_568,
                time_zone_key: "Synthetic zone".into(),
            });
            Ok(vec![result])
        },
    )
    .unwrap();
    assert_eq!(value["entries"][0]["metadata"]["attributes"], 6);
    assert_eq!(
        value["entries"][0]["metadata"]["lastWriteTimeLocalTicks100ns"],
        "638975574001234568"
    );
    assert_eq!(
        value["entries"][0]["metadata"]["lastWriteTimeUtc100ns"],
        "134064342001234568"
    );
}

#[test]
fn invalid_or_null_explicit_directory_options_never_query() {
    for fields in [
        json!({"order":null}),
        json!({"metadata":null}),
        json!({"order":"unknown"}),
        json!({"entry_type":"unknown"}),
        json!({"search_pattern":"*.txt"}),
        json!({"match_mode":"windows-find"}),
        json!({"search_pattern":"*","match_mode":"dotnet-managed"}),
        json!({"search_pattern":"sub\\*.txt","match_mode":"windows-find"}),
    ] {
        let mut value = params();
        for (key, item) in fields.as_object().unwrap() {
            value[key] = item.clone();
        }
        assert_error(
            read_directory_with(value, &CancellationToken::new(), deadline(), |_, _, _| {
                panic!("invalid options queried")
            })
            .unwrap_err(),
            "invalid_input",
        );
    }
    assert_error(
        read_directory_with(
            json!({"path":"C:/synthetic/","max_entries":2,"metadata":true}),
            &CancellationToken::new(),
            deadline(),
            |_, _, _| Ok(vec![entry("a")]),
        )
        .unwrap_err(),
        "fs_error",
    );
}
