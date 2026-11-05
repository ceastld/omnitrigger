// 仅合成描述，不启动宿主、读取用户工程或执行 JavaScript。
#[path = "../src/runtime_capabilities.rs"]
mod runtime_capabilities;
use runtime_capabilities::{validate_requirements, RuntimeDescription};
use serde_json::{json, Value};

fn old_host(engine: &str) -> Value {
    json!({"schema_version":1,"product":"omnitrigger-script-host","version":"0.1.0",
        "engine":engine,"api_version":1,"protocol_schema":1})
}
fn parse(value: &Value) -> RuntimeDescription {
    RuntimeDescription::parse(&serde_json::to_vec(value).unwrap()).unwrap()
}
fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| (*s).into()).collect()
}

#[test]
fn old_hosts_remain_compatible_without_explicit_requirements() {
    for engine in ["boa", "quickjs"] {
        let description = parse(&old_host(engine));
        assert_eq!(description.version, "0.1.0");
        assert_eq!(description.engine, engine);
        assert_eq!(description.capabilities, None);
        description.check(&[]).unwrap();
    }
}
#[test]
fn same_version_does_not_prove_new_api_support() {
    let old = parse(&old_host("boa"));
    let required = names(&["clipboard.history.v1", "process.capture.v1"]);
    let fault = old.check(&required).unwrap_err();
    assert_eq!(fault.code, "runtime_capabilities_unknown");
    assert_eq!(fault.effect_state, "not_started");
    assert_eq!(fault.missing_capabilities, required);
    let encoded = serde_json::to_value(fault).unwrap();
    assert_eq!(encoded["effect_state"], "not_started");
}
#[test]
fn declared_new_hosts_satisfy_only_exact_requested_contracts() {
    for engine in ["boa", "quickjs"] {
        let mut value = old_host(engine);
        value["capabilities"] = json!([
            "clipboard.history.v1",
            "process.capture.v1",
            "future.feature.v7"
        ]);
        let new = parse(&value);
        new.check(&names(&["clipboard.history.v1", "process.capture.v1"]))
            .unwrap();
        let fault = new
            .check(&names(&[
                "clipboard.history.v2",
                "process.capture",
                "future.feature.v7",
            ]))
            .unwrap_err();
        assert_eq!(fault.code, "runtime_capability_missing");
        assert_eq!(
            fault.missing_capabilities,
            names(&["clipboard.history.v2", "process.capture"])
        );
    }
}
#[test]
fn explicit_empty_list_is_distinct_from_legacy_unknown() {
    let mut value = old_host("boa");
    value["capabilities"] = json!([]);
    let description = parse(&value);
    description.check(&[]).unwrap();
    assert_eq!(
        description
            .check(&names(&["clipboard.history.v1"]))
            .unwrap_err()
            .code,
        "runtime_capability_missing"
    );
}
#[test]
fn invalid_capability_shapes_are_rejected_instead_of_guessed() {
    for bad in [
        json!(null),
        json!(true),
        json!(1),
        json!("history"),
        json!({}),
        json!([null]),
        json!(["x", "x"]),
        json!([""]),
        json!(["X"]),
        json!(["a/b"]),
        json!(["x".repeat(97)]),
        json!((0..33).map(|n| format!("cap.{n}")).collect::<Vec<_>>()),
    ] {
        let mut value = old_host("boa");
        value["capabilities"] = bad;
        assert_eq!(
            RuntimeDescription::parse(&serde_json::to_vec(&value).unwrap())
                .unwrap_err()
                .code,
            "runtime_description_invalid"
        );
    }
}
#[test]
fn malformed_requirements_fail_even_for_legacy_hosts() {
    let host = parse(&old_host("boa"));
    for invalid in [
        names(&[""]),
        names(&["x", "x"]),
        names(&["x\n"]),
        vec!["x".repeat(97)],
        (0..33).map(|n| format!("cap.{n}")).collect(),
    ] {
        assert_eq!(
            validate_requirements(&invalid).unwrap_err().code,
            "invalid_runtime_requirements"
        );
        assert_eq!(
            host.check(&invalid).unwrap_err().code,
            "invalid_runtime_requirements"
        );
    }
}
#[test]
fn identity_and_protocol_fail_without_echoing_untrusted_values() {
    for (key, bad) in [
        ("schema_version", json!(2)),
        ("product", json!("private-value")),
        ("api_version", json!(2)),
        ("protocol_schema", json!(2)),
        ("engine", json!("node")),
        ("version", json!("")),
        ("version", json!("x".repeat(65))),
    ] {
        let mut value = old_host("boa");
        value[key] = bad;
        let fault = RuntimeDescription::parse(&serde_json::to_vec(&value).unwrap()).unwrap_err();
        assert_eq!(fault.code, "runtime_description_invalid");
        assert!(!fault.message.contains("private-value"));
    }
}
#[test]
fn description_is_bounded_and_additive_fields_remain_compatible() {
    for invalid in [
        b"[]".to_vec(),
        b"null".to_vec(),
        b"{".to_vec(),
        vec![b' '; 4097],
    ] {
        assert!(RuntimeDescription::parse(&invalid).is_err());
    }
    let mut value = old_host("quickjs");
    value["future_metadata"] = json!({"x":1});
    parse(&value).check(&[]).unwrap();
}
