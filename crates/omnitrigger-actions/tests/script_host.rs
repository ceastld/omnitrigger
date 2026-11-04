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
    let output = runtime()
        .execute(
            &fixture.action(input.clone()),
            &context(),
            &CancellationToken::new(),
        )
        .unwrap();
    assert_eq!(
        output,
        json!({"input":input,"invocation":{"invocation_id":u64::MAX.to_string(),"binding_id":"9007199254740993"},"root":std::fs::canonicalize(&fixture.root).unwrap(),"version":1})
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
