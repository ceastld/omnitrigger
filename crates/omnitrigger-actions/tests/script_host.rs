//! 真正的轻量 JS 进程接入测试；CI 在两种引擎构建后显式运行 ignored 用例。
use omnitrigger_actions::{
    ActionCall, ActionDefinition, ActionError, ActionRuntime, CancellationToken, InvocationContext,
    NativeHost, ResolvedAction, ScriptHostRuntime,
};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    thread,
    time::{Duration, Instant},
};
static NEXT: AtomicU64 = AtomicU64::new(1);

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new(source: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "omnitrigger-script-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(
            root.join("main.js"),
            format!("function main(ctx) {{\n{source}\n}}"),
        )
        .unwrap();
        Self { root }
    }
    fn action(&self, input: Value) -> ResolvedAction {
        ResolvedAction {
            package_root: self.root.clone(),
            entry_path: self.root.join("main.js"),
            call: ActionCall {
                package_id: "fixture".into(),
                action_id: "run".into(),
                action_version: 1,
                input,
            },
            definition: ActionDefinition {
                id: "run".into(),
                title: "脚本接入样本".into(),
                version: 1,
                runtime: "script".into(),
                entry: "main.js".into(),
            },
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for name in ["main.js", "input.txt", "output.txt", "ready.txt"] {
            let _ = std::fs::remove_file(self.root.join(name));
        }
        let _ = std::fs::remove_dir(&self.root);
    }
}
fn runtime() -> ScriptHostRuntime {
    ScriptHostRuntime::new(PathBuf::from(
        std::env::var_os("OMNITRIGGER_TEST_SCRIPT_HOST").expect("设置真实宿主路径"),
    ))
}
fn context() -> InvocationContext {
    InvocationContext {
        invocation_id: u64::MAX,
        binding_id: 9_007_199_254_740_993,
    }
}

#[test]
fn unavailable_host_and_pre_cancel_are_rejected_before_execution() {
    let fixture = Fixture::new("return {};");
    let runtime = ScriptHostRuntime::new(fixture.root.join("does-not-exist.exe"));
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let cancelled = runtime
        .execute(&fixture.action(json!({})), &context(), &cancellation)
        .unwrap_err();
    assert_eq!(cancelled.code, "cancelled");
    assert_eq!(cancelled.effect_state.as_deref(), Some("not_started"));
    let unavailable = runtime
        .execute(
            &fixture.action(json!({})),
            &context(),
            &CancellationToken::new(),
        )
        .unwrap_err();
    assert_eq!(unavailable.code, "runtime_start_failed");
    assert_eq!(unavailable.effect_state.as_deref(), Some("not_started"));
}

#[test]
fn installed_image_digest_is_checked_before_any_process_starts() {
    let fixture = Fixture::new("return {};");
    let program = fixture.root.join("host.exe");
    std::fs::write(&program, b"not an executable").unwrap();
    for (digest, size) in [
        ("0".repeat(64), 17),
        ("invalid".into(), 17),
        ("0".repeat(64), 1),
    ] {
        let error = ScriptHostRuntime::new(&program)
            .with_integrity(digest, size)
            .execute(
                &fixture.action(json!({})),
                &context(),
                &CancellationToken::new(),
            )
            .unwrap_err();
        assert_eq!(error.code, "runtime_integrity");
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    }
    std::fs::remove_file(program).unwrap();
}

#[test]
#[ignore = "需要显式指定实际构建的 Boa 或 QuickJS 宿主"]
fn script_diagnostics_associate_entry_paths_without_changing_fault_fields() {
    let fixture =
        Fixture::new("function fail() {\n throw new Error('entry-diagnostic');\n}\nreturn fail();");
    let entry_path = fixture.root.join("main.js").display().to_string();
    let action = fixture.action(json!({}));
    let error = runtime()
        .execute(&action, &context(), &CancellationToken::new())
        .unwrap_err();
    assert_eq!(error.code, "script_failed");
    assert_eq!(error.effect_state.as_deref(), Some("unknown"));
    assert!(error.message.contains(&entry_path));
    assert!(error.message.contains("action.js:3"), "{}", error.message);
    std::fs::write(
        fixture.root.join("main.js"),
        "function main(ctx) {\n ctx.desktop.sendText(123);\n return null;\n}",
    )
    .unwrap();
    let error = runtime()
        .execute(&action, &context(), &CancellationToken::new())
        .unwrap_err();
    assert_eq!(error.code, "invalid_input");
    assert_eq!(error.method.as_deref(), Some("keyboard.text"));
    assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    assert!(error.message.contains("action.js:2"), "{}", error.message);
    std::fs::write(
        fixture.root.join("main.js"),
        "function main() { throw new Error('字'.repeat(20000)); }",
    )
    .unwrap();
    let error = runtime()
        .execute(&action, &context(), &CancellationToken::new())
        .unwrap_err();
    assert_eq!(error.code, "script_failed");
    assert!(error.message.len() <= 16 * 1024);
    assert!(error.message.contains(&entry_path));
    assert!(error.message.contains("action.js"));
}

#[test]
#[ignore = "需要显式指定实际构建的 Boa 或 QuickJS 宿主"]
fn package_root_supports_normal_javascript_path_concatenation() {
    let fixture = Fixture::new("const file = ctx.packageRoot + '/output.txt'; ctx.fs.writeText(file, '路径拼接验证'); return ctx.fs.readText(file);");
    let output = runtime()
        .execute(
            &fixture.action(json!({})),
            &context(),
            &CancellationToken::new(),
        )
        .unwrap();
    assert_eq!(output, json!("路径拼接验证"));
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("output.txt")).unwrap(),
        "路径拼接验证"
    );
}

#[test]
#[ignore = "需要显式指定实际构建的 Boa 或 QuickJS 宿主"]
fn script_delay_keeps_order_and_allows_cancellation_and_timeout() {
    let fixture = Fixture::new(
        "ctx.delay(30); ctx.fs.writeText(ctx.input.output,'after delay'); return {done:true};",
    );
    let output = fixture.root.join("output.txt");
    let began = Instant::now();
    assert_eq!(
        runtime()
            .execute(
                &fixture.action(json!({"output":output})),
                &context(),
                &CancellationToken::new()
            )
            .unwrap(),
        json!({"done":true})
    );
    assert!(began.elapsed() >= Duration::from_millis(30));
    assert_eq!(std::fs::read_to_string(&output).unwrap(), "after delay");
    std::fs::remove_file(&output).unwrap();

    std::fs::write(fixture.root.join("main.js"), "function main(ctx) {ctx.fs.writeText(ctx.input.ready,'before delay'); ctx.delay(60000); ctx.fs.writeText(ctx.input.output,'must not run'); return true;}").unwrap();
    let ready = fixture.root.join("ready.txt");
    let action = fixture.action(json!({"ready":ready,"output":output}));
    let cancellation = CancellationToken::new();
    let signal = cancellation.clone();
    let worker_action = action.clone();
    let worker = thread::spawn(move || runtime().execute(&worker_action, &context(), &signal));
    let began = Instant::now();
    while !ready.exists() {
        assert!(began.elapsed() < Duration::from_secs(30));
        thread::sleep(Duration::from_millis(10));
    }
    let began = Instant::now();
    cancellation.cancel();
    let error = worker.join().unwrap().unwrap_err();
    assert_eq!(error.code, "cancelled");
    assert_eq!(error.effect_state.as_deref(), Some("unknown"));
    assert!(began.elapsed() < Duration::from_secs(5));
    assert!(!output.exists());
    assert_eq!(std::fs::read_to_string(&ready).unwrap(), "before delay");
    std::fs::remove_file(&ready).unwrap();

    let error = runtime()
        .with_timeout(Duration::from_secs(2))
        .execute(&action, &context(), &CancellationToken::new())
        .unwrap_err();
    assert_eq!(error.code, "timeout");
    assert_eq!(error.effect_state.as_deref(), Some("unknown"));
    assert_eq!(std::fs::read_to_string(&ready).unwrap(), "before delay");
    assert!(!output.exists());
}

#[test]
#[ignore = "需要显式指定实际构建的 Boa 或 QuickJS 宿主"]
fn script_host_runs_generated_standard_js_project_without_repository_paths() {
    let fixture = Fixture::new("return null;");
    let root = fixture.root.join("actions");
    let project = omnitrigger_actions::project::create_script_project(&root, "generated").unwrap();
    let action = omnitrigger_actions::PackageCatalog::load(&root)
        .resolve(&project.call)
        .unwrap();
    let output = runtime()
        .execute(&action, &context(), &CancellationToken::new())
        .unwrap();
    assert_eq!(
        output,
        json!({"text":"OmniTrigger","invocation":{"invocation_id":u64::MAX.to_string(),"binding_id":"9007199254740993"}})
    );
    // 只清理本测试明确创建的路径；有额外文件时保留目录。
    for name in [
        "action.json",
        "main.js",
        "call.json",
        "jsconfig.json",
        "README.md",
        "sdk/api.d.ts",
    ] {
        std::fs::remove_file(project.package_root.join(name)).unwrap();
    }
    std::fs::remove_dir(project.package_root.join("sdk")).unwrap();
    std::fs::remove_dir(project.package_root).unwrap();
    std::fs::remove_dir(root).unwrap();
}

#[test]
#[ignore = "需要显式指定实际构建的 Boa 或 QuickJS 宿主"]
fn script_host_round_trip_has_real_context_without_node() {
    let fixture = Fixture::new("return {input:ctx.input, invocation:ctx.invocation, root:ctx.packageRoot, version:ctx.apiVersion};");
    let input = json!({"text":"中文 ' \" $(not-a-command)\n第二行","__proto__":{"polluted":true}});
    let mut output = runtime()
        .execute(
            &fixture.action(input.clone()),
            &context(),
            &CancellationToken::new(),
        )
        .unwrap();
    let root = output.as_object_mut().unwrap().remove("root").unwrap();
    let root = PathBuf::from(root.as_str().unwrap());
    assert!(root.is_absolute());
    assert_eq!(
        std::fs::canonicalize(&root).unwrap(),
        std::fs::canonicalize(&fixture.root).unwrap()
    );
    #[cfg(windows)]
    assert!(!root.to_string_lossy().starts_with(r"\\?\"));
    assert_eq!(
        output,
        json!({"input":input,"invocation":{"invocation_id":u64::MAX.to_string(),"binding_id":"9007199254740993"},"version":1})
    );
}

#[test]
#[ignore = "需要显式指定实际构建的 Boa 或 QuickJS 宿主"]
fn script_host_executes_file_api_and_rejects_accidental_overwrite() {
    let fixture = Fixture::new("const text=ctx.fs.readText(ctx.input.source); ctx.fs.writeText(ctx.input.destination,text.trim()); return {written:true};");
    std::fs::write(fixture.root.join("input.txt"), "  中文\r\n ").unwrap();
    let action = fixture.action(json!({"source":fixture.root.join("input.txt"),"destination":fixture.root.join("output.txt")}));
    assert_eq!(
        runtime()
            .execute(&action, &context(), &CancellationToken::new())
            .unwrap(),
        json!({"written":true})
    );
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("output.txt")).unwrap(),
        "中文"
    );
    let failed = runtime()
        .execute(&action, &context(), &CancellationToken::new())
        .unwrap_err();
    assert_eq!(failed.code, "script_failed");
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("output.txt")).unwrap(),
        "中文"
    );
}

#[test]
#[ignore = "需要显式指定实际构建的 Boa 或 QuickJS 宿主"]
fn script_host_cancel_and_timeout_do_not_claim_rollback() {
    let fixture = Fixture::new(&format!(
        "ctx.fs.writeText(ctx.input.ready,'effect remains'); while(true) {{ let x=1; {} }}",
        "x=Math.sin(x)+Math.cos(x);".repeat(32)
    ));
    let action = fixture.action(json!({"ready":fixture.root.join("ready.txt")}));
    let cancellation = CancellationToken::new();
    let signal = cancellation.clone();
    let worker = thread::spawn(move || runtime().execute(&action, &context(), &signal));
    let began = Instant::now();
    while !fixture.root.join("ready.txt").exists() {
        assert!(began.elapsed() < Duration::from_secs(30));
        thread::sleep(Duration::from_millis(10));
    }
    let cancel_began = Instant::now();
    cancellation.cancel();
    let error = worker.join().unwrap().unwrap_err();
    assert_eq!(error.code, "cancelled");
    assert_eq!(error.effect_state.as_deref(), Some("unknown"));
    assert!(cancel_began.elapsed() < Duration::from_secs(5));
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("ready.txt")).unwrap(),
        "effect remains"
    );
    // 用纯循环单独验证宿主时限，避免将 API 参数错误误认作超时。
    std::fs::write(fixture.root.join("main.js"), "while(true) {};").unwrap();
    let timed = runtime()
        .with_timeout(Duration::ZERO)
        .execute(
            &fixture.action(json!({})),
            &context(),
            &CancellationToken::new(),
        )
        .unwrap_err();
    assert_eq!(timed.code, "timeout");
    assert_eq!(timed.effect_state.as_deref(), Some("unknown"));
}

#[derive(Default)]
struct RecordingHost {
    calls: Mutex<Vec<(String, Value)>>,
    fail: bool,
}
impl NativeHost for RecordingHost {
    fn call(
        &self,
        method: &str,
        params: Value,
        _: &CancellationToken,
    ) -> Result<Value, ActionError> {
        self.calls.lock().unwrap().push((method.into(), params));
        if self.fail {
            let mut fault = ActionError::new("target_changed", "原目标已改变。");
            fault.effect_state = Some("not_started".into());
            return Err(fault);
        }
        Ok(match method {
            "target.info" => json!({"processName":"fixture.exe","isForeground":true}),
            "target.context" => {
                json!({"window_handle":"18446744073709551615","process_created_file_time":"9007199254740993"})
            }
            "clipboard.read" | "selection.copy" => json!("选区 中文😀"),
            "clipboard.write" => json!({"written":true,"sequenceNumber":42}),
            _ => json!({"submitted":true}),
        })
    }
}

#[test]
#[ignore = "需要显式指定实际构建的 Boa 或 QuickJS 宿主"]
fn script_line_edit_example_uses_original_target_and_preflights_parameters() {
    struct Host {
        process: &'static str,
        fail_second: bool,
        cancel_after_first: bool,
        calls: Mutex<Vec<(String, Value)>>,
    }
    impl NativeHost for Host {
        fn call(
            &self,
            method: &str,
            params: Value,
            cancellation: &CancellationToken,
        ) -> Result<Value, ActionError> {
            let mut calls = self.calls.lock().unwrap();
            calls.push((method.into(), params));
            if self.cancel_after_first && method == "keyboard.sequence" && calls.len() == 2 {
                cancellation.cancel();
            }
            if self.fail_second && method == "keyboard.sequence" && calls.len() == 3 {
                let mut error = ActionError::new("target_changed", "原目标已不在前台。");
                error.effect_state = Some("not_started".into());
                return Err(error);
            }
            Ok(if method == "target.info" {
                json!({"processName":self.process})
            } else {
                json!({"submitted":true})
            })
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/actions");
    let catalog = omnitrigger_actions::PackageCatalog::load(&root);
    let call: ActionCall = serde_json::from_str(include_str!(
        "../../../examples/actions/desktop-line-edit/call-script.json"
    ))
    .unwrap();
    for (process, input, delay) in [
        ("Code.exe", json!({}), 0),
        ("WPS.EXE", json!({"waitMs":0}), 10),
        ("mstsc.exe", json!({"waitMs":0}), 10),
        ("Code.exe", json!({"waitMs":0,"keyDownDelayMs":7}), 7),
        ("WPS.EXE", json!({"waitMs":0,"keyDownDelayMs":0}), 0),
    ] {
        let host = Host {
            process,
            fail_second: false,
            cancel_after_first: false,
            calls: Mutex::default(),
        };
        let mut call = call.clone();
        call.input = input;
        let output = runtime()
            .execute_with_host(
                &catalog.resolve(&call).unwrap(),
                &context(),
                &CancellationToken::new(),
                Some(&host),
            )
            .unwrap();
        assert_eq!(output, json!({"processName":process,"inputSubmitted":true}));
        assert_eq!(
            *host.calls.lock().unwrap(),
            vec![
                ("target.info".into(), json!({})),
                (
                    "keyboard.sequence".into(),
                    json!({"modifiers":[160],"keys":[36],"key_down_delay_ms":delay})
                ),
                (
                    "keyboard.sequence".into(),
                    json!({"modifiers":[],"keys":[8],"key_down_delay_ms":delay})
                ),
            ]
        );
    }
    for input in [
        json!({"waitMs":"80"}),
        json!({"waitMs":-1}),
        json!({"waitMs":60001}),
        json!({"waitMs":null}),
        json!({"keyDownDelayMs":"0"}),
        json!({"keyDownDelayMs":1001}),
    ] {
        let host = Host {
            process: "Code.exe",
            fail_second: false,
            cancel_after_first: false,
            calls: Mutex::default(),
        };
        let mut call = call.clone();
        call.input = input;
        let error = runtime()
            .execute_with_host(
                &catalog.resolve(&call).unwrap(),
                &context(),
                &CancellationToken::new(),
                Some(&host),
            )
            .unwrap_err();
        assert_eq!(error.code, "invalid_input");
        assert_eq!(error.method.as_deref(), Some("script.parameters"));
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        assert!(host.calls.lock().unwrap().is_empty());
    }
    let host = Host {
        process: "Code.exe",
        fail_second: true,
        cancel_after_first: false,
        calls: Mutex::default(),
    };
    let error = runtime()
        .execute_with_host(
            &catalog.resolve(&call).unwrap(),
            &context(),
            &CancellationToken::new(),
            Some(&host),
        )
        .unwrap_err();
    assert_eq!(error.code, "target_changed");
    assert_eq!(error.method.as_deref(), Some("keyboard.sequence"));
    assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    assert_eq!(host.calls.lock().unwrap().len(), 3);
    let host = Host {
        process: "Code.exe",
        fail_second: false,
        cancel_after_first: true,
        calls: Mutex::default(),
    };
    let error = runtime()
        .execute_with_host(
            &catalog.resolve(&call).unwrap(),
            &context(),
            &CancellationToken::new(),
            Some(&host),
        )
        .unwrap_err();
    assert_eq!(error.code, "cancelled");
    assert_eq!(host.calls.lock().unwrap().len(), 2);
}

#[test]
#[ignore = "需要显式指定实际构建的 Boa 或 QuickJS 宿主"]
fn script_keyboard_recipe_preserves_steps_timing_and_stops_after_faults() {
    struct Host {
        process: &'static str,
        fail_at: usize,
        cancel_at: usize,
        calls: Mutex<Vec<(String, Value)>>,
    }
    impl NativeHost for Host {
        fn call(
            &self,
            method: &str,
            params: Value,
            cancellation: &CancellationToken,
        ) -> Result<Value, ActionError> {
            let mut calls = self.calls.lock().unwrap();
            calls.push((method.into(), params));
            if calls.len() == self.cancel_at {
                cancellation.cancel();
            }
            if calls.len() == self.fail_at {
                let mut error = ActionError::new("target_changed", "原目标已改变。");
                error.effect_state = Some("not_started".into());
                return Err(error);
            }
            Ok(if method == "target.info" {
                json!({"processName":self.process})
            } else {
                json!({"submitted":true})
            })
        }
    }
    let make_host = |process| Host {
        process,
        fail_at: usize::MAX,
        cancel_at: usize::MAX,
        calls: Mutex::default(),
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/actions");
    let catalog = omnitrigger_actions::PackageCatalog::load(&root);
    let call: ActionCall = serde_json::from_str(include_str!(
        "../../../examples/actions/keyboard-sequence/call-script.json"
    ))
    .unwrap();
    for (process, delay_override, expected_delay) in [
        ("Code.exe", None, 0),
        ("WPS.EXE", None, 10),
        ("mstsc.exe", None, 10),
        ("WPS.EXE", Some(0), 0),
        ("Code.exe", Some(7), 7),
    ] {
        let mut call = call.clone();
        if let Some(delay) = delay_override {
            call.input["keyDownDelayMs"] = json!(delay);
        }
        let host = make_host(process);
        let began = Instant::now();
        let output = runtime()
            .execute_with_host(
                &catalog.resolve(&call).unwrap(),
                &context(),
                &CancellationToken::new(),
                Some(&host),
            )
            .unwrap();
        assert_eq!(output, json!({"inputSubmitted":true,"steps":4}));
        assert!(began.elapsed() >= Duration::from_millis(80));
        let mut expected = vec![];
        if delay_override.is_none() {
            expected.push(("target.info".into(), json!({})));
        }
        expected.extend([
            (
                "keyboard.sequence".into(),
                json!({"modifiers":[160],"keys":[36],"key_down_delay_ms":expected_delay}),
            ),
            (
                "keyboard.sequence".into(),
                json!({"modifiers":[],"keys":[8],"key_down_delay_ms":expected_delay}),
            ),
        ]);
        assert_eq!(*host.calls.lock().unwrap(), expected);
    }
    let mut mixed = call.clone();
    mixed.input = json!({"steps":[
        {"kind":"sequence","modifiers":[163],"keys":[77,77]},
        {"kind":"delay","milliseconds":0},
        {"kind":"tap","key":65},
        {"kind":"text","text":"中文😀${literal}"}
    ]});
    let host = make_host("WPS.EXE");
    runtime()
        .execute_with_host(
            &catalog.resolve(&mixed).unwrap(),
            &context(),
            &CancellationToken::new(),
            Some(&host),
        )
        .unwrap();
    assert_eq!(
        *host.calls.lock().unwrap(),
        vec![
            ("target.info".into(), json!({})),
            (
                "keyboard.sequence".into(),
                json!({"modifiers":[163],"keys":[77,77],"key_down_delay_ms":10})
            ),
            (
                "keyboard.sequence".into(),
                json!({"modifiers":[],"keys":[65],"key_down_delay_ms":0})
            ),
            ("keyboard.text".into(), json!({"text":"中文😀${literal}"})),
        ]
    );
    // 原目标在第二次输入前变化，后续文本不能提交；此前输入不能被当作已撤销。
    let mut host = make_host("Code.exe");
    host.fail_at = 3;
    let error = runtime()
        .execute_with_host(
            &catalog.resolve(&mixed).unwrap(),
            &context(),
            &CancellationToken::new(),
            Some(&host),
        )
        .unwrap_err();
    assert_eq!(error.code, "target_changed");
    assert_eq!(error.method.as_deref(), Some("keyboard.sequence"));
    assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    assert_eq!(host.calls.lock().unwrap().len(), 3);
    let mut host = make_host("Code.exe");
    host.cancel_at = 2;
    let error = runtime()
        .execute_with_host(
            &catalog.resolve(&mixed).unwrap(),
            &context(),
            &CancellationToken::new(),
            Some(&host),
        )
        .unwrap_err();
    assert_eq!(error.code, "cancelled");
    assert_eq!(host.calls.lock().unwrap().len(), 2);
}

#[test]
#[ignore = "需要显式指定实际构建的 Boa 或 QuickJS 宿主"]
fn script_keyboard_recipe_checks_late_parameters_and_host_call_budget() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/actions");
    let catalog = omnitrigger_actions::PackageCatalog::load(&root);
    let mut call: ActionCall = serde_json::from_str(include_str!(
        "../../../examples/actions/keyboard-sequence/call-script.json"
    ))
    .unwrap();
    for last in [
        json!(null),
        json!({"kind":"unknown"}),
        json!({"kind":"delay","milliseconds":"30"}),
        json!({"kind":"delay","milliseconds":-1}),
        json!({"kind":"delay","milliseconds":60001}),
        json!({"kind":"tap","key":0}),
        json!({"kind":"sequence","modifiers":[],"keys":[]}),
        json!({"kind":"text","text":null}),
        json!({"kind":"text","text":"bad\0text"}),
    ] {
        call.input = json!({"steps":[{"kind":"tap","key":65},last]});
        let host = RecordingHost::default();
        let error = runtime()
            .execute_with_host(
                &catalog.resolve(&call).unwrap(),
                &context(),
                &CancellationToken::new(),
                Some(&host),
            )
            .unwrap_err();
        assert_eq!(error.code, "invalid_input");
        assert_eq!(error.method.as_deref(), Some("script.parameters"));
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        assert!(host.calls.lock().unwrap().is_empty());
    }
    for (steps, delay) in [
        (json!([{"kind":"tap","key":65}]), json!("10")),
        (json!([{"kind":"tap","key":65}]), json!(1001)),
        (
            json!([{"kind":"tap","key":65},{"kind":"sequence","modifiers":[],"keys":[65,66,67]}]),
            json!(1000),
        ),
    ] {
        call.input = json!({"steps":steps,"keyDownDelayMs":delay});
        let host = RecordingHost::default();
        let error = runtime()
            .execute_with_host(
                &catalog.resolve(&call).unwrap(),
                &context(),
                &CancellationToken::new(),
                Some(&host),
            )
            .unwrap_err();
        assert_eq!(error.code, "invalid_input");
        assert_eq!(error.method.as_deref(), Some("script.parameters"));
        assert!(host.calls.lock().unwrap().is_empty());
    }
    call.input = json!({"steps":[{"kind":"delay","milliseconds":0}]});
    let host = RecordingHost::default();
    let output = runtime()
        .execute_with_host(
            &catalog.resolve(&call).unwrap(),
            &context(),
            &CancellationToken::new(),
            Some(&host),
        )
        .unwrap();
    assert_eq!(output, json!({"inputSubmitted":false,"steps":1}));
    assert!(host.calls.lock().unwrap().is_empty());
    // 32 步加一次默认目标查询会超预算；显式时序恰好允许 32 次调用。
    call.input = json!({"steps":vec![json!({"kind":"sequence","modifiers":[],"keys":[65]});32]});
    let host = RecordingHost::default();
    let error = runtime()
        .execute_with_host(
            &catalog.resolve(&call).unwrap(),
            &context(),
            &CancellationToken::new(),
            Some(&host),
        )
        .unwrap_err();
    assert_eq!(error.code, "invalid_input");
    assert!(host.calls.lock().unwrap().is_empty());
    call.input["keyDownDelayMs"] = json!(0);
    let output = runtime()
        .execute_with_host(
            &catalog.resolve(&call).unwrap(),
            &context(),
            &CancellationToken::new(),
            Some(&host),
        )
        .unwrap();
    assert_eq!(output, json!({"inputSubmitted":true,"steps":32}));
    let calls = host.calls.lock().unwrap();
    assert_eq!(calls.len(), 32);
    assert!(calls
        .iter()
        .all(|(method, _)| method == "keyboard.sequence"));
}

#[test]
#[ignore = "需要显式指定实际构建的 Boa 或 QuickJS 宿主"]
fn script_window_api_routes_operations_validates_and_preserves_faults() {
    let fixture = Fixture::new(
        r#"
        const rejected = [];
        for (const operation of [null, undefined, false, 0, {}, 'Close', 'maximize', 'resize']) {
            try { ctx.desktop.windowAction(operation); } catch(e) { rejected.push([e.code,e.effectState,e.method]); }
        }
        const receipts = ['toggle_maximize','minimize','close'].map(op => ctx.desktop.windowAction(op));
        return {rejected,receipts};
    "#,
    );
    let host = RecordingHost::default();
    let output = runtime()
        .execute_with_host(
            &fixture.action(json!({})),
            &context(),
            &CancellationToken::new(),
            Some(&host),
        )
        .unwrap();
    assert_eq!(
        output["rejected"],
        json!(vec![
            vec!["invalid_input", "not_started", "window.action"];
            8
        ])
    );
    assert_eq!(
        output["receipts"],
        json!([{ "submitted":true },{ "submitted":true },{ "submitted":true }])
    );
    assert_eq!(
        *host.calls.lock().unwrap(),
        ["toggle_maximize", "minimize", "close"]
            .into_iter()
            .map(|op| ("window.action".into(), json!({"operation":op})))
            .collect::<Vec<_>>()
    );
    let fixture = Fixture::new("return ctx.desktop.windowAction('close');");
    let missing = runtime()
        .execute(
            &fixture.action(json!({})),
            &context(),
            &CancellationToken::new(),
        )
        .unwrap_err();
    assert_eq!(missing.code, "desktop_unavailable");
    assert_eq!(missing.method.as_deref(), Some("window.action"));
    let host = RecordingHost {
        fail: true,
        ..Default::default()
    };
    let error = runtime()
        .execute_with_host(
            &fixture.action(json!({})),
            &context(),
            &CancellationToken::new(),
            Some(&host),
        )
        .unwrap_err();
    assert_eq!(error.code, "target_changed");
    assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    assert_eq!(error.method.as_deref(), Some("window.action"));
}

#[test]
#[ignore = "需要显式指定实际构建的 Boa 或 QuickJS 宿主"]
fn script_host_routes_all_desktop_apis_and_validates_before_native_dispatch() {
    let fixture = Fixture::new(
        r#"
        const rejected = [];
        for (const invalid of [
            () => ctx.desktop.sendShortcut([]),
            () => ctx.desktop.sendShortcut([65], {keyDownDelayMs:-1}),
            () => ctx.desktop.sendShortcut([65], {timing:{process_overrides:null}}),
            () => ctx.desktop.sendKeySequence([], Array(17).fill(65)),
            () => ctx.desktop.sendKeySequence([], [65,66,67], {keyDownDelayMs:1000}),
            () => ctx.desktop.sendText('\ud800'),
            () => ctx.desktop.sendText('\udfff'),
            () => ctx.desktop.sendText('x\0'),
            () => ctx.clipboard.writeText('x', {excludeFromHistory:1}),
        ]) {
            try { invalid(); } catch(e) { rejected.push([e.code,e.effectState]); }
        }
        const target=ctx.desktop.target();
        const snapshot=ctx.desktop.targetContext();
        ctx.desktop.sendShortcut([162,67], {timing:{key_down_delay_ms:20,process_overrides:{'fixture.exe':30}}});
        ctx.desktop.sendKeySequence([162], [65,67], {keyDownDelayMs:10});
        ctx.desktop.sendText('中文😀');
        ctx.desktop.sendText('\u0001'.repeat(65536));
        const selection=ctx.desktop.copySelectionText();
        const text=ctx.clipboard.readText();
        const receipt=ctx.clipboard.writeText('临时😀', {excludeFromHistory:true});
        return {rejected,target,snapshot,selection,text,receipt};
    "#,
    );
    let host = RecordingHost::default();
    let result = runtime()
        .execute_with_host(
            &fixture.action(json!({})),
            &context(),
            &CancellationToken::new(),
            Some(&host),
        )
        .unwrap();
    assert_eq!(
        result["rejected"],
        json!(vec![vec!["invalid_input", "not_started"]; 9])
    );
    assert_eq!(result["target"]["processName"], "fixture.exe");
    assert_eq!(result["snapshot"]["window_handle"], u64::MAX.to_string());
    assert_eq!(result["selection"], "选区 中文😀");
    assert_eq!(result["text"], "选区 中文😀");
    assert_eq!(result["receipt"]["sequenceNumber"], 42);
    let calls = host.calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .map(|(method, _)| method.as_str())
            .collect::<Vec<_>>(),
        vec![
            "target.info",
            "target.context",
            "keyboard.shortcut",
            "keyboard.sequence",
            "keyboard.text",
            "keyboard.text",
            "selection.copy",
            "clipboard.read",
            "clipboard.write"
        ]
    );
    assert_eq!(
        calls[2].1,
        json!({"keys":[162,67],"timing":{"key_down_delay_ms":20,"process_overrides":{"fixture.exe":30}}})
    );
    assert_eq!(
        calls[3].1,
        json!({"modifiers":[162],"keys":[65,67],"key_down_delay_ms":10})
    );
    assert_eq!(calls[4].1, json!({"text":"中文😀"}));
    assert_eq!(calls[5].1["text"].as_str().unwrap().len(), 65536);
    assert_eq!(
        calls[8].1,
        json!({"text":"临时😀","exclude_from_history":true})
    );
}

#[test]
#[ignore = "需要显式指定实际构建的 Boa 或 QuickJS 宿主"]
fn script_host_preserves_native_faults_and_never_substitutes_an_ambient_target() {
    let fixture = Fixture::new("ctx.desktop.sendText('x'); return {};");
    let missing = runtime()
        .execute(
            &fixture.action(json!({})),
            &context(),
            &CancellationToken::new(),
        )
        .unwrap_err();
    assert_eq!(missing.code, "desktop_unavailable");
    assert_eq!(missing.effect_state.as_deref(), Some("not_started"));
    let host = RecordingHost {
        fail: true,
        ..Default::default()
    };
    let fault = runtime()
        .execute_with_host(
            &fixture.action(json!({})),
            &context(),
            &CancellationToken::new(),
            Some(&host),
        )
        .unwrap_err();
    assert_eq!(fault.code, "target_changed");
    assert_eq!(fault.effect_state.as_deref(), Some("not_started"));
    assert_eq!(fault.method.as_deref(), Some("keyboard.text"));
    assert_eq!(host.calls.lock().unwrap().len(), 1);
    let caught = Fixture::new("try {ctx.desktop.sendText('x')} catch(e) {return {code:e.code,method:e.method,effect:e.effectState}};");
    let result = runtime()
        .execute_with_host(
            &caught.action(json!({})),
            &context(),
            &CancellationToken::new(),
            Some(&host),
        )
        .unwrap();
    assert_eq!(
        result,
        json!({"code":"target_changed","method":"keyboard.text","effect":"not_started"})
    );
    assert_eq!(host.calls.lock().unwrap().len(), 2);
}

#[test]
#[ignore = "需要显式指定实际构建的 Boa 或 QuickJS 宿主"]
fn script_host_waits_for_native_io_and_cancellation_ends_pending_calls() {
    struct WaitingHost(std::sync::Arc<std::sync::atomic::AtomicBool>);
    impl NativeHost for WaitingHost {
        fn call(
            &self,
            _: &str,
            params: Value,
            cancellation: &CancellationToken,
        ) -> Result<Value, ActionError> {
            if params["text"] == "wait-for-cancel" {
                self.0.store(true, Ordering::SeqCst);
                let began = Instant::now();
                while !cancellation.is_cancelled() && began.elapsed() < Duration::from_secs(4) {
                    thread::sleep(Duration::from_millis(5));
                }
                return Err(ActionError::new("cancelled", "已取消。"));
            }
            thread::sleep(Duration::from_millis(2100));
            Ok(json!({"submitted":true}))
        }
    }
    // 原生等待不占 JS 的 CPU 时间配额；返回后仍能完成 JSON 结果。
    let fixture = Fixture::new("return ctx.desktop.sendText('slow');");
    assert_eq!(
        runtime()
            .execute_with_host(
                &fixture.action(json!({})),
                &context(),
                &CancellationToken::new(),
                Some(&WaitingHost(std::sync::Arc::default()))
            )
            .unwrap(),
        json!({"submitted":true})
    );
    let fixture = Fixture::new("return ctx.desktop.sendText('wait-for-cancel');");
    let cancellation = CancellationToken::new();
    let signal = cancellation.clone();
    let action = fixture.action(json!({}));
    let ready = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let host = WaitingHost(std::sync::Arc::clone(&ready));
    let worker = thread::spawn(move || {
        runtime().execute_with_host(&action, &context(), &signal, Some(&host))
    });
    let began = Instant::now();
    while !ready.load(Ordering::SeqCst) {
        assert!(began.elapsed() < Duration::from_secs(30));
        thread::sleep(Duration::from_millis(5));
    }
    cancellation.cancel();
    let began = Instant::now();
    let error = worker.join().unwrap().unwrap_err();
    assert_eq!(error.code, "cancelled");
    assert_eq!(error.effect_state.as_deref(), Some("unknown"));
    assert!(began.elapsed() < Duration::from_secs(3));
}

#[test]
#[ignore = "需要显式指定实际构建的 Boa 或 QuickJS 宿主"]
fn script_host_browser_api_keeps_large_values_undefined_and_permission_metadata() {
    struct BrowserMock(Mutex<Vec<(String, Value)>>);
    impl NativeHost for BrowserMock {
        fn call(
            &self,
            method: &str,
            params: Value,
            _: &CancellationToken,
        ) -> Result<Value, ActionError> {
            self.0.lock().unwrap().push((method.into(), params.clone()));
            if method == "browser.profiles" {
                return Ok(
                    json!([{"browser_instance_id":"work","session_id":"s","name":"fixture"}]),
                );
            }
            assert_eq!(method, "browser.call");
            Ok(match params["method"].as_str().unwrap() {
                "bookmarks.getTree" => {
                    json!({"has_value":true,"value":[{"title":"中文😀".repeat(110000)}]})
                }
                "test.undefined" => json!({"has_value":false}),
                "test.null" => json!({"has_value":true,"value":null}),
                "test.permission" => {
                    let mut error = ActionError::new("permission_required", "需要书签权限。");
                    error.method = Some("test.permission".into());
                    error.effect_state = Some("not_started".into());
                    error.required_permissions = vec!["bookmarks".into()];
                    return Err(error);
                }
                _ => panic!("unexpected method"),
            })
        }
    }
    let host = BrowserMock(Mutex::new(Vec::new()));
    let fixture = Fixture::new(
        r#"
        let rejected=0;
        for(const invalid of [
            ()=>ctx.browser.call('constructor.x'),
            ()=>ctx.browser.call('chrome.tabs.query'),
            ()=>ctx.browser.call('tabs.query',[undefined]),
            ()=>ctx.browser.call('tabs.query',[NaN]),
            ()=>ctx.browser.call('tabs.query',[{text:'\ud800'}]),
            ()=>ctx.browser.call('tabs.query',[],{timeoutMs:0}),
            ()=>ctx.browser.call('tabs.query',{},{}),
            ()=>ctx.browser.call('tabs.query',[],{profile:'work',unknown:true}),
            ()=>ctx.browser.call('tabs.query',['x'.repeat(530000)]),
        ]) {try{invalid()}catch(error){if(error.effectState==='not_started') rejected++;}}
        const profiles=ctx.browser.profiles();
        const tree=ctx.browser.chrome.bookmarks.getTree();
        const missing=ctx.browser.call('test.undefined',[],{profile:'work',timeoutMs:5000})===undefined;
        const isNull=ctx.browser.call('test.null')===null;
        let fault;
        try{ctx.browser.call('test.permission')}catch(error){fault={code:error.code,effect:error.effectState,method:error.method,permissions:error.requiredPermissions};}
        return {rejected,profile:profiles[0].browser_instance_id,characters:tree[0].title.length,missing,isNull,fault};
    "#,
    );
    let result = runtime()
        .execute_with_host(
            &fixture.action(json!({})),
            &context(),
            &CancellationToken::new(),
            Some(&host),
        )
        .unwrap();
    assert_eq!(
        result,
        json!({"rejected":9,"profile":"work","characters":440000,"missing":true,"isNull":true,"fault":{"code":"permission_required","effect":"not_started","method":"test.permission","permissions":["bookmarks"]}})
    );
    let calls = host.0.lock().unwrap();
    assert_eq!(calls.len(), 5);
    assert_eq!(
        calls[2].1,
        json!({"method":"test.undefined","args":[],"options":{"profile":"work","timeoutMs":5000}})
    );
    drop(calls);
    let fixture = Fixture::new("ctx.browser.call('test.permission'); return {};");
    let error = runtime()
        .execute_with_host(
            &fixture.action(json!({})),
            &context(),
            &CancellationToken::new(),
            Some(&host),
        )
        .unwrap_err();
    assert_eq!(error.code, "permission_required");
    assert_eq!(error.method.as_deref(), Some("test.permission"));
    assert_eq!(error.required_permissions, vec!["bookmarks"]);
    assert_eq!(host.0.lock().unwrap().len(), 6);
}
