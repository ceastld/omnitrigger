//! 真实NodeRuntime与假NativeHost的协议集成；不读取UIA、输入或剪贴板。
//! CI显式传入OMNITRIGGER_SELECTION_TEST_NODE并运行ignored，缺少Node直接失败。
use omnitrigger_actions::{
    ActionCall, ActionDefinition, ActionError, ActionRuntime, CancellationToken, InvocationContext,
    NativeHost, NodeRuntime, ResolvedAction,
};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};
static NEXT: AtomicU64 = AtomicU64::new(1);
fn local(path: &std::path::Path) {
    assert!(path.is_absolute(), "需要完整本地路径");
    for part in path.ancestors() {
        assert!(
            !part.file_name().is_some_and(|v| {
                let n = v.to_string_lossy().to_ascii_lowercase();
                n.starts_with("onedrive") || n == "dropbox" || n.starts_with("icloud")
            }),
            "跳过云盘目录"
        );
        let meta = fs::symlink_metadata(part).unwrap();
        assert!(!meta.file_type().is_symlink());
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            assert_eq!(meta.file_attributes() & 0x441400, 0);
        }
    }
}
fn runtime() -> NodeRuntime {
    let path = PathBuf::from(
        std::env::var_os("OMNITRIGGER_SELECTION_TEST_NODE")
            .expect("显式测试需要OMNITRIGGER_SELECTION_TEST_NODE，不允许静默跳过"),
    );
    local(&path);
    assert!(path.is_file());
    NodeRuntime::new(path)
}
struct Fixture {
    root: PathBuf,
    action: ResolvedAction,
}
impl Fixture {
    fn new(source: &str) -> Self {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        local(&repo);
        let parent = repo.join(".tmp");
        if !parent.exists() {
            fs::create_dir(&parent).unwrap();
        }
        local(&parent);
        let root = parent.join(format!(
            "selection-read-node-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        local(&root);
        let entry_path = root.join("main.mjs");
        fs::write(&entry_path, source).unwrap();
        let action = ResolvedAction {
            call: ActionCall {
                package_id: "test.selection-read".into(),
                action_id: "read".into(),
                action_version: 1,
                input: json!({}),
            },
            definition: ActionDefinition {
                id: "read".into(),
                title: "只读选区测试".into(),
                version: 1,
                runtime: "node".into(),
                entry: "main.mjs".into(),
                single_instance: false,
            },
            package_root: root.clone(),
            entry_path,
        };
        Self { root, action }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.root.join("main.mjs"));
        let _ = fs::remove_dir(&self.root);
    }
}
fn original() -> Value {
    json!({"window_handle":"101","process_id":10,"thread_id":11,"process_created_file_time":"134000000000000001","expires_at_unix_ms":1000})
}
fn error(code: &str) -> ActionError {
    let mut e = ActionError::new(code, "PRIVATE synthetic provider message");
    e.effect_state = Some("not_started".into());
    e
}
struct Host {
    calls: Mutex<Vec<(String, Value)>>,
    replies: Mutex<VecDeque<Result<Value, ActionError>>>,
    cancel: Option<CancellationToken>,
}
impl Host {
    fn new(replies: Vec<Result<Value, ActionError>>) -> Self {
        Self {
            calls: Mutex::default(),
            replies: Mutex::new(replies.into()),
            cancel: None,
        }
    }
}
impl NativeHost for Host {
    fn call(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError> {
        self.calls.lock().unwrap().push((method.into(), params));
        match method {
            "target.context" => Ok(original()),
            "selection.read_text" => {
                if let Some(outer) = &self.cancel {
                    outer.cancel();
                    cancellation.cancel();
                    return Err(error("cancelled"));
                }
                self.replies
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("出现额外读取或重试")
            }
            _ => panic!("不能发送输入、调用Clipboard或重新捕获目标：{method}"),
        }
    }
}
fn run(fixture: &Fixture, host: &Host, token: &CancellationToken) -> Result<Value, ActionError> {
    runtime().execute_with_host(
        &fixture.action,
        &InvocationContext {
            invocation_id: 1,
            binding_id: 2,
        },
        token,
        Some(host),
    )
}
#[test]
#[ignore = "显式NodeRuntime+假NativeHost测试；不运行UIA或GUI"]
fn empty_success_and_provider_errors_cross_the_real_node_bridge() {
    let f = Fixture::new(
        r#"export default async function(ctx){const values=[await ctx.desktop.readSelectionText(),await ctx.desktop.readSelectionText({overflow:'truncate'})];const errors=[];for(let i=0;i<3;i++){try{await ctx.desktop.readSelectionText();}catch(e){errors.push({code:e.code,effect:e.effectState,leaked:e.message.includes('PRIVATE')});}}return{values,errors};}"#,
    );
    let host = Host::new(vec![
        Ok(json!({"text":"","truncated":false})),
        Ok(json!({"text":"","truncated":false})),
        Err(error("selection_unavailable")),
        Err(error("target_changed")),
        Err(error("timeout")),
    ]);
    let output = run(&f, &host, &CancellationToken::new()).unwrap();
    assert_eq!(
        output["values"],
        json!([{"text":"","truncated":false},{"text":"","truncated":false}])
    );
    assert_eq!(
        output["errors"],
        json!([{"code":"selection_unavailable","effect":"not_started","leaked":false},{"code":"target_changed","effect":"not_started","leaked":false},{"code":"timeout","effect":"not_started","leaked":false}])
    );
    let calls = host.calls.lock().unwrap();
    assert_eq!(calls.len(), 5);
    assert_eq!(calls[0].1, json!({}));
    assert_eq!(calls[1].1, json!({"overflow":"truncate"}));
    assert!(calls.iter().all(|c| c.0 == "selection.read_text"));
}
#[test]
#[ignore = "显式NodeRuntime+假NativeHost测试；不运行UIA或GUI"]
fn strict_unicode_and_overflow_receipts_cross_the_real_node_bridge() {
    let f = Fixture::new(
        r#"export default async function(ctx){const good=await ctx.desktop.readSelectionText({overflow:'truncate'});const errors=[];for(let i=0;i<4;i++){try{await ctx.desktop.readSelectionText({overflow:i===3?'reject':'truncate'});}catch(e){errors.push(e.code);}}return{good,errors};}"#,
    );
    let host = Host::new(vec![
        Ok(json!({"text":"😀".repeat(64),"truncated":true})),
        Ok(json!({"text":"A".repeat(129),"truncated":false})),
        Ok(json!({"text":"\0","truncated":false})),
        Ok(json!({"text":"","truncated":true})),
        Ok(json!({"text":"A".repeat(128),"truncated":true})),
    ]);
    let output = run(&f, &host, &CancellationToken::new()).unwrap();
    assert_eq!(
        output["good"],
        json!({"text":"😀".repeat(64),"truncated":true})
    );
    assert_eq!(
        output["errors"],
        json!([
            "invalid_response",
            "invalid_response",
            "invalid_response",
            "invalid_response"
        ])
    );
    assert_eq!(host.calls.lock().unwrap().len(), 5);
}
#[test]
#[ignore = "显式NodeRuntime+假NativeHost测试；不运行UIA或GUI"]
fn options_and_original_context_are_snapshots_through_the_real_node_bridge() {
    let f = Fixture::new(
        r#"export default async function(ctx){const before=await ctx.desktop.targetContext();const options={overflow:'truncate'};const pending=ctx.desktop.readSelectionText(options);options.overflow='reject';const result=await pending;const after=await ctx.desktop.targetContext();return{before,after,result,frozen:Object.isFrozen(result)};}"#,
    );
    let host = Host::new(vec![Ok(json!({"text":"","truncated":false}))]);
    let output = run(&f, &host, &CancellationToken::new()).unwrap();
    assert_eq!(output["before"], original());
    assert_eq!(output["after"], original());
    assert_eq!(output["frozen"], true);
    assert_eq!(
        *host.calls.lock().unwrap(),
        vec![
            (String::from("target.context"), json!({})),
            (
                String::from("selection.read_text"),
                json!({"overflow":"truncate"})
            ),
            (String::from("target.context"), json!({}))
        ]
    );
}
#[test]
#[ignore = "显式NodeRuntime取消测试；不运行UIA或GUI"]
fn cancelled_selection_does_not_continue_to_keyboard_or_clipboard() {
    let f = Fixture::new(
        r#"export default async function(ctx){await ctx.desktop.readSelectionText();await ctx.desktop.sendText('must not send');return 'must not succeed';}"#,
    );
    let token = CancellationToken::new();
    let mut host = Host::new(vec![]);
    host.cancel = Some(token.clone());
    assert_eq!(run(&f, &host, &token).unwrap_err().code, "cancelled");
    let calls = host.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "selection.read_text");
}

#[test]
#[ignore = "显式 NodeRuntime 与假 NativeHost；不运行 UIA/GUI"]
fn explicit_max_units_crosses_actual_node_runtime_with_worst_case_json() {
    let fixture = Fixture::new(
        r#"export default async ctx=>await ctx.desktop.readSelectionText({maxUtf16Units:8192,overflow:'truncate'});"#,
    );
    let text = "\u{1}".repeat(8192);
    let reply = json!({"text":text,"truncated":true});
    assert!(serde_json::to_vec(&reply).unwrap().len() < 64 * 1024);
    let host = Host::new(vec![Ok(reply.clone())]);
    assert_eq!(
        run(&fixture, &host, &CancellationToken::new()).unwrap(),
        reply
    );
    assert_eq!(
        *host.calls.lock().unwrap(),
        vec![(
            "selection.read_text".into(),
            json!({"max_utf16_units":8192,"overflow":"truncate"})
        )]
    );
}
