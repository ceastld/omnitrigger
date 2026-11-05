//! 纯内存提交边界测试，不启动 Node 或其它进程。
use super::*;
use std::cell::Cell;

#[derive(Debug)]
struct HostGuard<'a>(&'a Cell<u32>);
impl Drop for HostGuard<'_> {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

#[test]
fn startup_exhaustion_drops_host_without_writing_action() {
    let began = Instant::now();
    let deadline = began + Duration::from_secs(60);
    let now = Cell::new(began);
    let dropped = Cell::new(0);
    let cancellation = CancellationToken::new();
    let mut bytes = Vec::new();
    let result = start_before_request(
        deadline,
        &cancellation,
        || now.get(),
        || {
            now.set(deadline);
            Ok(HostGuard(&dropped))
        },
    )
    .and_then(|_host| {
        write_request(&mut bytes, json!({}), deadline, &cancellation, || now.get()).map(|_| ())
    });
    let fault = result.unwrap_err();
    assert_eq!(fault.code, "timeout");
    assert_eq!(fault.effect_state.as_deref(), Some("not_started"));
    assert!(bytes.is_empty());
    assert_eq!(dropped.get(), 1);
}

#[test]
fn cancellation_during_start_drops_host_without_writing_action() {
    let began = Instant::now();
    let deadline = began + Duration::from_secs(60);
    let dropped = Cell::new(0);
    let cancellation = CancellationToken::new();
    let mut bytes = Vec::new();
    let result = start_before_request(
        deadline,
        &cancellation,
        || began,
        || {
            cancellation.cancel();
            Ok(HostGuard(&dropped))
        },
    )
    .and_then(|_host| {
        write_request(&mut bytes, json!({}), deadline, &cancellation, || began).map(|_| ())
    });
    let fault = result.unwrap_err();
    assert_eq!(fault.code, "cancelled");
    assert_eq!(fault.effect_state.as_deref(), Some("not_started"));
    assert!(bytes.is_empty());
    assert_eq!(dropped.get(), 1);
}

#[test]
fn delayed_writer_and_zero_budget_do_not_write_a_frame() {
    let began = Instant::now();
    let cancellation = CancellationToken::new();
    let mut bytes = Vec::new();
    let fault = write_request(&mut bytes, json!({}), began, &cancellation, || began).unwrap_err();
    assert_eq!(fault.code, "timeout");
    assert_eq!(fault.effect_state.as_deref(), Some("not_started"));
    let deadline = began + Duration::from_secs(60);
    let clock_calls = Cell::new(0);
    let fault = write_request(&mut bytes, json!({}), deadline, &cancellation, || {
        let instant = if clock_calls.get() == 0 {
            began
        } else {
            deadline
        };
        clock_calls.set(clock_calls.get() + 1);
        instant
    })
    .unwrap_err();
    assert_eq!(fault.code, "timeout");
    assert_eq!(fault.effect_state.as_deref(), Some("not_started"));
    assert!(bytes.is_empty());
    cancellation.cancel();
    let fault = write_request(
        &mut bytes,
        json!({}),
        began + Duration::from_secs(60),
        &cancellation,
        || began,
    )
    .unwrap_err();
    assert_eq!(fault.code, "cancelled");
    assert_eq!(fault.effect_state.as_deref(), Some("not_started"));
    assert!(bytes.is_empty());
}

#[test]
fn request_carries_only_remaining_budget_and_keeps_original_payload() {
    let began = Instant::now();
    let deadline = began + Duration::from_secs(60);
    let cancellation = CancellationToken::new();
    let request = json!({
        "api_version":1,"timeout_ms":60000,
        "invocation":{"invocation_id":"18446744073709551615","binding_id":"9"},
        "input":{"text":"固定输入"}
    });
    let bytes = write_request(Vec::new(), request.clone(), deadline, &cancellation, || {
        began + Duration::from_secs(30)
    })
    .unwrap();
    assert_eq!(bytes.last(), Some(&b'\n'));
    let frame: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(frame["timeout_ms"], 30000);
    assert_eq!(frame["input"], request["input"]);
    assert_eq!(frame["invocation"], request["invocation"]);
    assert_eq!(frame["api_version"], 1);
    assert_eq!(
        remaining_before_request(deadline, &cancellation, began + Duration::from_secs(50)).unwrap(),
        Duration::from_secs(10)
    );
}
