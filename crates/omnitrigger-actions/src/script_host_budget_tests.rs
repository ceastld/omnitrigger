//! 不启动进程；通过同一提交检查注入时钟，验证动作请求和自有宿主的边界。
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
fn startup_exhausting_budget_drops_host_without_submitting_request() {
    let began = Instant::now();
    let deadline = began + Duration::from_secs(60);
    let now = Cell::new(began);
    let dropped = Cell::new(0);
    let cancellation = CancellationToken::new();
    let mut input = Vec::new();
    let result = start_before_request(
        deadline,
        &cancellation,
        || now.get(),
        || {
            now.set(deadline);
            Ok(HostGuard(&dropped))
        },
    )
    .and_then(|_host| write_request(&mut input, b"action", deadline, &cancellation, now.get()));
    let fault = result.unwrap_err();
    assert_eq!(fault.code, "timeout");
    assert_eq!(fault.effect_state.as_deref(), Some("not_started"));
    assert!(input.is_empty());
    assert_eq!(dropped.get(), 1);
}

#[test]
fn cancellation_during_start_drops_host_without_submitting_request() {
    let began = Instant::now();
    let deadline = began + Duration::from_secs(60);
    let dropped = Cell::new(0);
    let cancellation = CancellationToken::new();
    let mut input = Vec::new();
    let result = start_before_request(
        deadline,
        &cancellation,
        || began,
        || {
            cancellation.cancel();
            Ok(HostGuard(&dropped))
        },
    )
    .and_then(|_host| write_request(&mut input, b"action", deadline, &cancellation, began));
    let fault = result.unwrap_err();
    assert_eq!(fault.code, "cancelled");
    assert_eq!(fault.effect_state.as_deref(), Some("not_started"));
    assert!(input.is_empty());
    assert_eq!(dropped.get(), 1);
}

#[test]
fn delayed_writer_rechecks_deadline_and_cancellation_before_writing() {
    let began = Instant::now();
    let deadline = began + Duration::from_secs(60);
    let cancellation = CancellationToken::new();
    let mut input = Vec::new();
    let fault =
        write_request(&mut input, b"action", deadline, &cancellation, deadline).unwrap_err();
    assert_eq!(fault.code, "timeout");
    assert_eq!(fault.effect_state.as_deref(), Some("not_started"));
    assert!(input.is_empty());
    cancellation.cancel();
    let fault = write_request(&mut input, b"action", deadline, &cancellation, began).unwrap_err();
    assert_eq!(fault.code, "cancelled");
    assert_eq!(fault.effect_state.as_deref(), Some("not_started"));
    assert!(input.is_empty());
}

#[test]
fn preparation_start_and_execution_share_deadline_and_keep_success_response() {
    let began = Instant::now();
    let deadline = began + Duration::from_secs(60);
    let now = Cell::new(began + Duration::from_secs(10));
    let cancellation = CancellationToken::new();
    assert_eq!(
        remaining_budget(deadline, &cancellation, now.get(), false).unwrap(),
        Duration::from_secs(50)
    );
    let dropped = Cell::new(0);
    let host = start_before_request(
        deadline,
        &cancellation,
        || now.get(),
        || {
            now.set(began + Duration::from_secs(30));
            Ok(HostGuard(&dropped))
        },
    )
    .unwrap();
    assert_eq!(
        remaining_budget(deadline, &cancellation, now.get(), true).unwrap(),
        Duration::from_secs(30)
    );
    let mut input = Vec::new();
    write_request(&mut input, b"action", deadline, &cancellation, now.get()).unwrap();
    assert_eq!(input.as_slice(), b"action");
    assert_eq!(
        parse_response(
            br#"{"schema_version":1,"ok":true,"result":{"done":true}}"#,
            true
        )
        .unwrap(),
        json!({"done":true})
    );
    let fault = remaining_budget(deadline, &cancellation, deadline, true).unwrap_err();
    assert_eq!(fault.code, "timeout");
    assert_eq!(fault.effect_state.as_deref(), Some("unknown"));
    cancellation.cancel();
    let fault = remaining_budget(deadline, &cancellation, now.get(), true).unwrap_err();
    assert_eq!(fault.code, "cancelled");
    assert_eq!(fault.effect_state.as_deref(), Some("unknown"));
    drop(host);
    assert_eq!(dropped.get(), 1);
}
