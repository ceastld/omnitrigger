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
fn action_context_metadata_preserves_full_integer_identity() {
    let root = std::env::current_dir().unwrap();
    let invocation =
        json!({"invocation_id":"18446744073709551615","binding_id":"9007199254740993"});
    let (ok, response) = invoke(
        json!({"script":"return { invocation:ctx.invocation, root:ctx.packageRoot };", "input":{}, "invocation":invocation,"package_root":root}),
    );
    assert!(ok, "{response}");
    assert_eq!(response["schema_version"], 1);
    assert_eq!(
        response["result"],
        json!({"invocation":invocation,"root":root})
    );
    let (ok, _) = invoke(
        json!({"script":"return 1;","input":{},"invocation":{"invocation_id":"not-an-id","binding_id":"0"}}),
    );
    assert!(!ok);
}

#[test]
fn desktop_requires_explicit_context_and_keeps_bridge_failure_structured() {
    let (ok, response) =
        invoke(json!({"script":"return ctx.browser.call('tabs.query');","input":{}}));
    assert!(!ok);
    assert_eq!(response["error"]["code"], "browser_unavailable");
    assert_eq!(response["error"]["effect_state"], "not_started");
    assert_eq!(response["error"]["method"], "tabs.query");
    let script = "ctx.desktop.sendText('x'); return {};";
    let (ok, response) = invoke(json!({"script":script,"input":{}}));
    assert!(!ok);
    assert_eq!(response["error"]["code"], "desktop_unavailable");
    assert_eq!(response["error"]["effect_state"], "not_started");
    // 桥接初始化失败时剪贴板也不能退回宿主自己的系统 API。
    let (ok, response) = invoke(
        json!({"script":"ctx.clipboard.writeText('must not write'); return {};","input":{},"native_context":{"error":{"code":"native_bridge_unavailable","message":"fixture unavailable","effect_state":"not_started"}}}),
    );
    assert!(!ok);
    assert_eq!(response["error"]["code"], "native_bridge_unavailable");
    assert_eq!(response["error"]["method"], "clipboard.write");
    let (ok, response) = invoke(
        json!({"script":"try {ctx.desktop.target()} catch(e) {return {code:e.code,effect:e.effectState,method:e.method}}","input":{}}),
    );
    assert!(ok);
    assert_eq!(
        response["result"],
        json!({"code":"desktop_unavailable","effect":"not_started","method":"target.info"})
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

#[test]
fn native_file_api_completes_a_real_transformation_and_preserves_existing_files() {
    let directory = std::env::temp_dir().join(format!(
        "omnitrigger-api-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    let source = directory.join("输入 中文.txt");
    let target = directory.join("输出 中文.txt");
    std::fs::write(&source, "第一行  \r\n第二行\t\n").unwrap();
    let (ok, response) = invoke(json!({
        "script": include_str!("../examples/clean-file.js"),
        "input": {"source":source,"destination":target}
    }));
    assert!(ok, "{response}");
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "第一行\n第二行\n"
    );
    let (ok, response) = invoke(
        json!({"script":"try { ctx.fs.writeText(ctx.input.path, '覆盖'); } catch(e) { return {code:e.code}; }", "input":{"path":target}}),
    );
    assert!(ok, "{response}");
    assert_eq!(response["result"]["code"], "file_write_failed");
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "第一行\n第二行\n"
    );
    let (ok, response) = invoke(
        json!({"script":"ctx.fs.writeText(ctx.input.path, '已替换', {overwrite:true}); return ctx.fs.readText(ctx.input.path);", "input":{"path":target}}),
    );
    assert!(ok, "{response}");
    assert_eq!(response["result"], "已替换");
    std::fs::remove_file(source).unwrap();
    std::fs::remove_file(target).unwrap();
    std::fs::remove_dir(directory).unwrap();
}

#[test]
fn native_api_rejects_invalid_arguments_with_stable_error_codes() {
    for (script, code) in [
        ("ctx.fs.readText('relative.txt')", "invalid_path"),
        ("ctx.fs.writeText(42, 'x')", "invalid_arguments"),
        (
            "ctx.fs.writeText('C:/unused.txt', 'x'.repeat(65537))",
            "text_too_large",
        ),
        ("ctx.process.run('', [])", "invalid_arguments"),
        (
            "ctx.process.run('not-used', [], {timeoutMs:0})",
            "invalid_arguments",
        ),
        (
            "ctx.process.start('omnitrigger-nonexistent-program-api-test')",
            "process_start_failed",
        ),
        ("ctx.open('javascript:alert(1)')", "open_failed"),
    ] {
        let (ok, response) = invoke(
            json!({"script":format!("try {{ {script}; }} catch (e) {{ return {{code:e.code}}; }} throw new Error('应当拒绝');"),"input":{}}),
        );
        assert!(ok, "{script}: {response}");
        assert_eq!(response["result"]["code"], code, "{script}: {response}");
    }
}

#[cfg(windows)]
#[test]
fn native_process_api_reports_real_exit_status_and_timeout_without_shell_guessing() {
    let program = std::env::var("COMSPEC").unwrap();
    let (ok, response) = invoke(
        json!({"script":"return ctx.process.run(ctx.input.program, ['/d','/c','exit','7']);", "input":{"program":program}}),
    );
    assert!(ok, "{response}");
    assert_eq!(response["result"]["exitCode"], 7);
    assert_eq!(response["result"]["success"], false);
    // 被测程序仍是宿主自己；不给 stdin 会立即报告解析失败，证明参数不被拼成 Shell 命令。
    let (ok, response) = invoke(
        json!({"script":"return ctx.process.run(ctx.input.program, ['a b & exit 0']);", "input":{"program":env!("CARGO_BIN_EXE_omnitrigger-js-probe")}}),
    );
    assert!(ok, "{response}");
    assert_eq!(response["result"]["exitCode"], 1);
    let (ok, response) = invoke(
        json!({"script":"return ctx.process.start(ctx.input.program, ['/d','/c','exit','0']);", "input":{"program":program}}),
    );
    assert!(ok, "{response}");
    assert!(response["result"]["pid"]
        .as_u64()
        .is_some_and(|pid| pid > 0));
    let sleeper = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let (ok, response) = invoke(
        json!({"script":"try { ctx.process.run(ctx.input.program, ['-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 5'], {timeoutMs:50}); } catch(e) { return {code:e.code}; } throw new Error('应当超时');", "input":{"program":sleeper}}),
    );
    assert!(ok, "{response}");
    assert_eq!(response["result"]["code"], "process_timeout");
}
