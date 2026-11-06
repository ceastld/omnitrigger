use super::*;
struct NoFallback;
impl NativeHost for NoFallback {
    fn call(&self, method: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
        panic!("目录枚举不能转交其他桌面适配器：{method}")
    }
}
#[test]
fn directory_method_is_dispatched_before_any_ui_or_inner_host_work() {
    let host = Host {
        inner: NoFallback,
        target: None,
    };
    let error = host
        .call_with_deadline(
            "fs.read_directory",
            serde_json::json!({"path":"relative","max_entries":1}),
            &CancellationToken::new(),
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap_err();
    assert_eq!(error.code, "invalid_input");
    assert_eq!(error.method.as_deref(), Some("fs.read_directory"));
    assert_eq!(error.effect_state.as_deref(), Some("not_started"));
}
