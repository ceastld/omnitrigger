//! 真正的轻量 JS 进程接入测试；CI 在两种引擎构建后显式运行 ignored 用例。
use omnitrigger_actions::{
    ActionCall, ActionDefinition, ActionRuntime, CancellationToken, InvocationContext,
    ResolvedAction, ScriptHostRuntime,
};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
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
