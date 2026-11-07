// 纯函数测试：不运行宿主，不打开 stdin，不初始化 JS 引擎。
#[path = "../../../crates/omnitrigger-actions/src/runtime_capabilities.rs"]
mod runtime_capabilities;
#[path = "../src/runtime_description.rs"]
mod runtime_description;

#[test]
fn current_descriptor_is_bounded_and_parses_as_a_new_host() {
    let value = runtime_description::describe();
    let bytes = serde_json::to_vec(&value).unwrap();
    assert!(bytes.len() < 4096);
    let host = runtime_capabilities::RuntimeDescription::parse(&bytes).unwrap();
    assert_eq!(host.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(
        host.engine,
        if cfg!(feature = "quickjs") {
            "quickjs"
        } else {
            "boa"
        }
    );
    assert_eq!(
        host.capabilities.as_ref().unwrap().len(),
        if cfg!(windows) { 8 } else { 5 }
    );
    host.check(&[
        "actions.stop_other_instances.v1".into(),
        "state.read_cas.v1".into(),
        "clipboard.history.v1".into(),
        "clipboard.read_data.v1".into(),
        "desktop.copy_selection_data.v1".into(),
    ])
    .unwrap();
    for capability in [
        "process.capture.v1",
        "system.open_file_in_parent.v1",
        "ui.action_hint.v1",
    ] {
        let result = host.check(&[capability.into()]);
        assert_eq!(result.is_ok(), cfg!(windows), "{capability}");
        if let Err(fault) = result {
            assert_eq!(fault.code, "runtime_capability_missing");
            assert_eq!(fault.effect_state, "not_started");
        }
    }
}
