use serde_json::{json, Value};
use std::{
    io::{ErrorKind, Write},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

fn invoke(request: Value) -> (bool, Value) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_omnitrigger-js-probe"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let written = child
        .stdin
        .take()
        .unwrap()
        .write_all(request.to_string().as_bytes());
    // 超限请求可能在发送完毕前被宿主拒绝；仍需读取真实错误回执。
    if let Err(error) = written {
        assert_eq!(error.kind(), ErrorKind::BrokenPipe);
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("脚本宿主未在 10 秒内退出");
        }
        thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    (
        output.status.success(),
        serde_json::from_slice(&output.stdout).unwrap(),
    )
}

#[test]
fn parameters_and_modern_javascript_round_trip() {
    let (ok, response) = invoke(json!({
        "script": "const {items, text} = ctx.input; return {sum: items.map(x => x * 2).reduce((a,b) => a+b, 0), text, big: String(42n), optional: ctx.input.absent?.x ?? '默认'};",
        "input": {"items": [1,2,3], "text": "中文\n\"\\\u{2028}"}
    }));
    assert!(ok, "{response}");
    assert_eq!(
        response["result"],
        json!({"sum":12, "text":"中文\n\"\\\u{2028}", "big":"42", "optional":"默认"})
    );
}

#[test]
fn prototype_keys_remain_plain_input_data() {
    let (ok, response) = invoke(
        json!({"script": "return {own: Object.hasOwn(ctx.input, '__proto__'), polluted: ({}).polluted ?? false};", "input": {"__proto__": {"polluted":true}}}),
    );
    assert!(ok, "{response}");
    assert_eq!(response["result"], json!({"own":true,"polluted":false}));
}

#[test]
fn failures_and_unsupported_results_are_explicit() {
    for script in [
        "throw new Error('测试失败')",
        "return undefined",
        "return 1n",
        "return Promise.resolve(1)",
        "return {",
        "while(true) {}",
        "function recurse() { return recurse(); } return recurse();",
        "return 'x'.repeat(70000)",
    ] {
        let (ok, response) = invoke(json!({"script":script,"input":{}}));
        assert!(!ok, "{script}: {response}");
        assert_eq!(response["ok"], false);
        assert!(response["error"]
            .as_str()
            .is_some_and(|error| !error.is_empty()));
    }
}

#[test]
fn invalid_requests_are_rejected_before_execution() {
    for request in [
        json!({"script":"return 1", "input":[]}),
        json!({"script":"return 1", "input":{},"extra":true}),
        json!({"script":"x".repeat(270000), "input":{}}),
    ] {
        let (ok, response) = invoke(request);
        assert!(!ok, "{response}");
    }
}
