//! 真实NodeRuntime与假NativeHost的协议集成；不读取Shell COM、输入或剪贴板。
//! CI显式传入OMNITRIGGER_EXPLORER_TEST_NODE并运行ignored，缺少Node直接失败。
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
        std::env::var_os("OMNITRIGGER_EXPLORER_TEST_NODE")
            .expect("显式测试需要OMNITRIGGER_EXPLORER_TEST_NODE，不允许静默跳过"),
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
            "explorer-context-node-{}-{}-{}",
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
                package_id: "test.explorer-context".into(),
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
            "explorer.context" => {
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
#[ignore = "显式NodeRuntime+假NativeHost；不执行Shell COM或GUI"]
fn explorer_context_crosses_real_node_bridge_and_preserves_original_context() {
    let f = Fixture::new(
        r#"export default async function(ctx){const before=await ctx.desktop.targetContext();const value=await ctx.desktop.explorerContext();const after=await ctx.desktop.targetContext();return{before,after,value,frozen:Object.isFrozen(value)&&Object.isFrozen(value.selectedPaths)};}"#,
    );
    let reply = json!({"currentDirectory":"D:\\fixture","selectedPaths":["D:\\fixture\\A 😀.txt"]});
    let host = Host::new(vec![Ok(reply.clone())]);
    let output = run(&f, &host, &CancellationToken::new()).unwrap();
    assert_eq!(output["value"], reply);
    assert_eq!(output["before"], original());
    assert_eq!(output["after"], original());
    assert_eq!(output["frozen"], true);
    assert_eq!(
        *host.calls.lock().unwrap(),
        vec![
            ("target.context".into(), json!({})),
            ("explorer.context".into(), json!({})),
            ("target.context".into(), json!({}))
        ]
    );
}
#[test]
#[ignore = "显式NodeRuntime+假NativeHost；不执行Shell COM或GUI"]
fn empty_selection_virtual_item_and_helper_failure_are_explicit() {
    let f = Fixture::new(
        r#"export default async function(ctx){const empty=await ctx.desktop.explorerContext();const errors=[];for(let i=0;i<3;i++){try{await ctx.desktop.explorerContext();}catch(e){errors.push({code:e.code,effect:e.effectState,private:e.message.includes('PRIVATE')});}}return{empty,errors};}"#,
    );
    let host = Host::new(vec![
        Ok(json!({"currentDirectory":null,"selectedPaths":[]})),
        Err(error("unsupported_item")),
        Err(error("explorer_helper_cleanup_failed")),
        Err(error("target_changed")),
    ]);
    let output = run(&f, &host, &CancellationToken::new()).unwrap();
    assert_eq!(
        output["empty"],
        json!({"currentDirectory":null,"selectedPaths":[]})
    );
    assert_eq!(
        output["errors"],
        json!([{"code":"unsupported_item","effect":"not_started","private":false},{"code":"explorer_helper_cleanup_failed","effect":"not_started","private":false},{"code":"target_changed","effect":"not_started","private":false}])
    );
    assert!(host
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|(method, params)| method == "explorer.context" && *params == json!({})));
}
