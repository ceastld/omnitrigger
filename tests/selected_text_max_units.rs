use super::*;
fn target() -> Value {
    json!({"window_handle":"1","process_id":2,"thread_id":3,"process_created_file_time":"4","expires_at_unix_ms":1000})
}
fn captured(value: Value) -> omnitrigger_windows::ProcessCaptureResult {
    omnitrigger_windows::ProcessCaptureResult {
        code: 0,
        stdout: value.to_string(),
        stderr: String::new(),
    }
}
fn success(text: &str, limit: u32, truncated: bool) -> Value {
    json!({"status":"ok","v":2,"target":target(),"overflow":"truncate","max_utf16_units":limit,"text":text,"truncated":truncated})
}
#[test]
fn real_options_and_cli_validate_limit_without_changing_defaults_or_budget() {
    assert_eq!(options(json!({})).unwrap().max_utf16_units, 128);
    for limit in [1, 128, 129, 2048, 8192] {
        assert_eq!(
            options(json!({"max_utf16_units":limit}))
                .unwrap()
                .max_utf16_units,
            limit
        );
        let payload = json!({"v":2,"budget_ms":2000,"target":target(),"overflow":"reject","max_utf16_units":limit});
        assert!(
            parse(&["self.exe".into(), FLAG.into(), payload.to_string()])
                .unwrap()
                .is_ok()
        );
    }
    for limit in [
        json!(0),
        json!(8193),
        json!(65536),
        json!(1.5),
        json!("8192"),
        json!(null),
    ] {
        assert_eq!(
            options(json!({"max_utf16_units":limit})).unwrap_err().code,
            "invalid_input"
        );
        let payload = json!({"v":2,"budget_ms":2000,"target":target(),"overflow":"reject","max_utf16_units":limit});
        assert!(
            parse(&["self.exe".into(), FLAG.into(), payload.to_string()])
                .unwrap()
                .is_err()
        );
    }
    assert_eq!(MAX_MS, 2000);
}
#[test]
fn actual_reply_decoder_binds_the_requested_limit_and_accepts_empty() {
    for (text, limit, truncated) in [
        ("字".repeat(2048), 2048, true),
        ("😀".repeat(4096), 8192, false),
        (String::new(), 8192, false),
    ] {
        assert_eq!(
            decode_with_limit(
                captured(success(&text, limit, truncated)),
                &target(),
                Overflow::Truncate,
                limit
            )
            .unwrap(),
            json!({"text":text,"truncated":truncated})
        );
    }
    for value in [
        success("short", 8192, true),
        success(&"A".repeat(8193), 8192, false),
        success("ok", 4096, false),
    ] {
        assert_eq!(
            decode_with_limit(captured(value), &target(), Overflow::Truncate, 8192)
                .unwrap_err()
                .code,
            "invalid_response"
        );
    }
    let mut missing = success("ok", 8192, false);
    missing.as_object_mut().unwrap().remove("max_utf16_units");
    assert_eq!(
        decode_with_limit(captured(missing), &target(), Overflow::Truncate, 8192)
            .unwrap_err()
            .code,
        "invalid_response"
    );
}
#[test]
fn serialized_worst_case_reply_is_bounded_by_the_actual_decoder_and_capture() {
    let value = Reply::Ok {
        v: 2,
        target: target(),
        overflow: Overflow::Truncate,
        max_utf16_units: 8192,
        text: "\u{1}".repeat(8192),
        truncated: true,
    };
    let bytes = serde_json::to_vec(&value).unwrap();
    assert!(bytes.len() + 1 <= reply_limit(8192));
    assert!(reply_limit(8192) < 64 * 1024);
    assert_eq!(reply_limit(128), 2048);
    let mut too_large_default = captured(json!({}));
    too_large_default.stdout = " ".repeat(2049);
    assert_eq!(
        decode(too_large_default, &target(), Overflow::Reject)
            .unwrap_err()
            .code,
        "selection_helper_failed"
    );
    assert!(decode_with_limit(
        captured(serde_json::from_slice(&bytes).unwrap()),
        &target(),
        Overflow::Truncate,
        8192
    )
    .is_ok());
    let mut oversized = captured(json!({}));
    oversized.stdout = " ".repeat(reply_limit(8192) + 1);
    assert_eq!(
        decode_with_limit(oversized, &target(), Overflow::Truncate, 8192)
            .unwrap_err()
            .code,
        "selection_helper_failed"
    );
}
#[test]
fn default_reply_keeps_v2_shape_and_old_error_codes_remain_errors() {
    let value = Reply::Ok {
        v: 2,
        target: target(),
        overflow: Overflow::Reject,
        max_utf16_units: 128,
        text: String::new(),
        truncated: false,
    };
    let value = serde_json::to_value(value).unwrap();
    assert!(value.get("max_utf16_units").is_none());
    assert_eq!(
        decode(captured(value), &target(), Overflow::Reject).unwrap(),
        json!({"text":"","truncated":false})
    );
    for code in [
        "selection_empty",
        "selection_too_long",
        "timeout",
        "cancelled",
        "target_changed",
    ] {
        assert_eq!(
            decode_with_limit(
                captured(json!({"status":"error","v":2,"code":code})),
                &target(),
                Overflow::Reject,
                8192
            )
            .unwrap_err()
            .code,
            code
        );
    }
}
