//! 真实 NodeRuntime 与假 NativeHost 的目录协议集成；不读取真实目录、UIA、输入或剪贴板。
//! CI显式传入OMNITRIGGER_DIRECTORY_TEST_NODE并运行ignored，缺少Node直接失败。
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
        std::env::var_os("OMNITRIGGER_DIRECTORY_TEST_NODE")
            .expect("显式测试需要OMNITRIGGER_DIRECTORY_TEST_NODE，不允许静默跳过"),
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
            "directory-read-node-{}-{}-{}",
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
                package_id: "test.directory-read".into(),
                action_id: "read".into(),
                action_version: 1,
                input: json!({}),
            },
            definition: ActionDefinition {
                id: "read".into(),
                title: "只读目录测试".into(),
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
struct Host {
    calls: Mutex<Vec<(String, Value)>>,
    replies: Mutex<VecDeque<Result<Value, ActionError>>>,
}
impl NativeHost for Host {
    fn call(
        &self,
        method: &str,
        params: Value,
        _: &CancellationToken,
    ) -> Result<Value, ActionError> {
        assert_eq!(method, "fs.read_directory");
        self.calls.lock().unwrap().push((method.into(), params));
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("出现额外调用或重试")
    }
}
fn host(replies: Vec<Result<Value, ActionError>>) -> Host {
    Host {
        calls: Mutex::default(),
        replies: Mutex::new(replies.into()),
    }
}
fn run(fixture: &Fixture, host: &Host) -> Result<Value, ActionError> {
    runtime().execute_with_host(
        &fixture.action,
        &InvocationContext {
            invocation_id: 1,
            binding_id: 2,
        },
        &CancellationToken::new(),
        Some(host),
    )
}
#[test]
#[ignore = "显式 NodeRuntime 与假 NativeHost；不读真实目录或桌面"]
fn directory_crosses_real_node_runtime_and_native_dispatcher() {
    let f = Fixture::new(
        r#"export default async ctx=>{const first=ctx.fs.readDirectory('X:/synthetic-only/',{maxEntries:2});return {promise:typeof first.then==='function',result:await first};}"#,
    );
    let reply = json!({"entries":[{"name":"a.txt","path":"X:/synthetic-only/a.txt","kind":"file","reason":null}],"complete":true});
    let h = host(vec![Ok(reply.clone())]);
    assert_eq!(run(&f, &h).unwrap(), json!({"promise":true,"result":reply}));
    assert_eq!(
        *h.calls.lock().unwrap(),
        vec![(
            "fs.read_directory".into(),
            json!({"path":"X:/synthetic-only/","max_entries":2})
        )]
    );
}
#[test]
#[ignore = "显式 NodeRuntime 与假 NativeHost；不读真实目录或桌面"]
fn directory_native_errors_preserve_code_without_leaking_provider_text() {
    let f = Fixture::new(
        r#"export default async ctx=>{const errors=[];for(let i=0;i<3;i++){try{await ctx.fs.readDirectory('X:/synthetic-only/');}catch(e){errors.push({code:e.code,method:e.method,effect:e.effectState,leaked:e.message.includes('PRIVATE')});}}return errors;}"#,
    );
    let codes = ["directory_too_large", "path_missing", "access_denied"];
    let h = host(
        codes
            .iter()
            .map(|code| Err(ActionError::new(*code, "PRIVATE provider path")))
            .collect(),
    );
    let expected:Vec<Value> = codes.iter().map(|code|json!({"code":code,"method":"fs.read_directory","effect":"not_started","leaked":false})).collect();
    assert_eq!(run(&f, &h).unwrap(), json!(expected));
    assert_eq!(h.calls.lock().unwrap().len(), 3);
}
#[test]
#[ignore = "显式 NodeRuntime 与假 NativeHost；不读真实目录或桌面"]
fn directory_invalid_path_and_wire_shape_do_not_pass_as_success() {
    let f = Fixture::new(
        r#"export default async ctx=>{let lexical;try{await ctx.fs.readDirectory('C:/a/../b');}catch(e){lexical=e.code;}try{await ctx.fs.readDirectory('X:/synthetic-only/');}catch(e){return {lexical,shape:e.code};}}"#,
    );
    let h = host(vec![Ok(json!({"entries":[],"complete":false}))]);
    assert_eq!(
        run(&f, &h).unwrap(),
        json!({"lexical":"invalid_input","shape":"fs_error"})
    );
    assert_eq!(h.calls.lock().unwrap().len(), 1);
}
