use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

static NEXT: AtomicU64 = AtomicU64::new(1);
struct Project(PathBuf);
impl Project {
    fn new(entry: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "omni-modules-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let project = Self(root);
        project.write("main.mjs", entry);
        project
    }
    fn write(&self, file: &str, source: &str) {
        let path = self.0.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    fn run(&self) -> (bool, Value) {
        invoke(json!({"module_entry":"main.mjs","package_root":self.0,"input":{"text":"  test  "}}))
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        assert_eq!(self.0.parent(), Some(std::env::temp_dir().as_path()));
        assert!(self
            .0
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("omni-modules-"));
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn invoke(request: Value) -> (bool, Value) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_omnitrigger-js-probe"));
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(request.to_string().as_bytes())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("模块宿主未在 10 秒内退出");
        }
        thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    let response = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&output.stderr)));
    (output.status.success(), response)
}

fn failure(project: &Project, code: &str) -> Value {
    let (ok, response) = project.run();
    assert!(!ok, "{response}");
    assert_eq!(response["error"]["code"], code, "{response}");
    assert!(matches!(
        response["error"]["effect_state"].as_str(),
        Some("not_started" | "unknown")
    ));
    response
}

#[test]
fn native_esm_imports_reexports_and_sdk_calls_round_trip() {
    let project = Project::new("import { clean } from './lib/index.js'; export function main(ctx) {return {text:clean(ctx.input.text),api:typeof ctx.fs.readText};}");
    project.write("lib/index.js", "export {clean} from './text.mjs';");
    project.write("lib/text.mjs", "export const clean = text => text.trim();");
    // 未导入文件不得被扫描或解析。
    project.write("unrelated.js", "not valid javascript {{{");
    let (ok, response) = project.run();
    assert!(ok, "{response}");
    assert_eq!(response["result"], json!({"text":"test","api":"function"}));
}

#[test]
fn normalized_import_identity_initializes_once_and_each_invocation_reloads() {
    let project = Project::new("import {count as a} from './counter.js'; import {count as b} from './lib/../counter.js'; export function main(){return {a,b};}");
    project.write("counter.js", "globalThis.initializations=(globalThis.initializations??0)+1; export const count=globalThis.initializations;");
    // 词法归一化不要求无关的中间目录存在。
    for _ in 0..2 {
        let (ok, response) = project.run();
        assert!(ok, "{response}");
        assert_eq!(response["result"], json!({"a":1,"b":1}));
    }
    project.write("counter.js", "export const count=42;");
    assert_eq!(project.run().1["result"], json!({"a":42,"b":42}));
}

#[test]
fn cycles_use_native_live_bindings_and_tdz_semantics() {
    let project = Project::new("import {readB,changeB} from './a.js'; export function main(){const before=readB();changeB();return [before,readB()];}");
    project.write("a.js", "import {b,setB} from './b.js'; export function readB(){return b;} export function changeB(){setB(3);}");
    project.write("b.js", "import {readB} from './a.js'; export let b=2; export function setB(x){b=x;} export function throughCycle(){return readB();}");
    let (ok, response) = project.run();
    assert!(ok, "{response}");
    assert_eq!(response["result"], json!([2, 3]));
    project.write("a.js", "import {b} from './b.js'; export const a=b; export const readB=()=>b; export function changeB(){}");
    project.write("b.js", "import {a} from './a.js'; export const b=a;");
    assert!(!project.run().0);
}

#[test]
fn imports_cannot_escape_the_package_or_use_host_module_names() {
    let project = Project::new("export function main(){return 1;}");
    for (specifier, code) in [
        ("../outside.js", "module_path"),
        ("./lib/../../outside.js", "module_path"),
        ("node:fs", "module_specifier"),
        ("fs", "module_specifier"),
        ("https://example.test/x.js", "module_specifier"),
        ("file:///x.js", "module_specifier"),
        ("/x.js", "module_specifier"),
        ("C:/x.js", "module_specifier"),
        ("./x.js?query", "module_specifier"),
        ("./x.js#fragment", "module_specifier"),
        ("./%2e%2e/x.js", "module_specifier"),
        ("./x", "module_specifier"),
        ("./x.json", "module_specifier"),
        ("./CON.js", "module_specifier"),
        ("./CON .js", "module_specifier"),
        ("./LPT¹.js", "module_specifier"),
        ("./CONIN$.js", "module_specifier"),
        ("./x\\y.js", "module_specifier"),
    ] {
        project.write(
            "main.mjs",
            &format!(
                "import {{x}} from {}; export function main(){{return x;}}",
                serde_json::to_string(specifier).unwrap()
            ),
        );
        failure(&project, code);
    }
}

#[test]
fn missing_invalid_and_oversized_modules_report_the_source_file() {
    let project =
        Project::new("import {x} from './lib/value.js'; export function main(){return x;}");
    let missing = failure(&project, "module_unavailable");
    assert!(missing["error"]["message"]
        .as_str()
        .unwrap()
        .contains("value.js"));
    project.write("lib/value.js", "export const x = ;");
    let syntax = failure(&project, "module_parse");
    assert!(syntax["error"]["message"]
        .as_str()
        .unwrap()
        .contains("value.js"));
    project.write(
        "lib/value.js",
        &format!("export const x=1;/*{}*/", "x".repeat(128 * 1024)),
    );
    failure(&project, "module_limit");
    fs::write(project.0.join("lib/value.js"), [0xff, 0xfe]).unwrap();
    failure(&project, "module_unavailable");
}

#[test]
fn count_and_total_source_limits_are_enforced() {
    let project = Project::new("export function main(){return 1;}");
    let mut imports = String::new();
    for index in 0..64 {
        project.write(&format!("file-{index}.js"), "export const x=1;");
        imports.push_str(&format!("import './file-{index}.js';\n"));
    }
    project.write(
        "main.mjs",
        &format!("{imports}export function main(){{return 1;}}"),
    );
    failure(&project, "module_limit");
    let padding = "x".repeat(120 * 1024);
    imports.clear();
    for index in 0..9 {
        project.write(&format!("file-{index}.js"), &format!("/*{padding}*/"));
        imports.push_str(&format!("import './file-{index}.js';\n"));
    }
    project.write(
        "main.mjs",
        &format!("{imports}export function main(){{return 1;}}"),
    );
    failure(&project, "module_limit");
}

#[test]
fn entry_contract_and_unresolved_initialization_are_explicit() {
    let project = Project::new("export default function main(){return 1;}");
    failure(&project, "module_entry");
    project.write("main.mjs", "export const main=1;");
    failure(&project, "module_entry");
    project.write("main.mjs", "export async function main(){return 1;}");
    assert!(!project.run().0);
    project.write(
        "main.mjs",
        "await new Promise(()=>{}); export function main(){return 1;}",
    );
    failure(&project, "module_pending");
    let (_, response) = invoke(
        json!({"module_entry":"main.mjs","script":"return 1;","package_root":project.0,"input":{}}),
    );
    assert_eq!(response["error"]["code"], "module_entry");
    let (_, response) =
        invoke(json!({"module_entry":"main.mjs","script":"","package_root":project.0,"input":{}}));
    assert_eq!(response["error"]["code"], "module_entry");
    let (_, response) = invoke(json!({"module_entry":"main.mjs","input":{}}));
    assert_eq!(response["error"]["code"], "module_entry");
    let (_, response) =
        invoke(json!({"module_entry":"../main.mjs","package_root":project.0,"input":{}}));
    assert_eq!(response["error"]["code"], "module_path");
}

#[test]
fn module_runtime_errors_locate_helpers_and_keep_native_fault_fields() {
    let project = Project::new(
        "import {fail} from './lib/error.js'; export function main(ctx){return fail(ctx);}",
    );
    project.write(
        "lib/error.js",
        "export function fail(ctx){\nthrow new Error('helper failed');\n}",
    );
    let (ok, response) = project.run();
    assert!(!ok);
    assert!(
        response["error"].to_string().contains("error.js"),
        "{response}"
    );
    project.write(
        "lib/error.js",
        "export function fail(ctx){return ctx.delay(-1);}",
    );
    let (ok, response) = project.run();
    assert!(!ok);
    assert_eq!(response["error"]["code"], "invalid_input");
    assert_eq!(response["error"]["effect_state"], "not_started");
    assert_eq!(response["error"]["method"], "delay");
}

#[test]
fn cloud_directory_names_are_rejected_before_opening_a_dependency() {
    let project =
        Project::new("import './OneDrive-test/value.js'; export function main(){return 1;}");
    // 该目录不存在，也不创建云盘名称目录；拒绝应发生在打开文件之前。
    failure(&project, "module_nonlocal");
}

#[test]
fn legacy_function_body_protocol_is_unchanged() {
    let (ok, response) = invoke(
        json!({"script":"function main(ctx){return {text:ctx.input.text.trim()};} return main(ctx);","input":{"text":" test "}}),
    );
    assert!(ok, "{response}");
    assert_eq!(response["result"], json!({"text":"test"}));
}

#[cfg(unix)]
#[test]
fn linked_files_and_directories_are_rejected_even_within_the_package() {
    use std::os::unix::fs::symlink;
    let project = Project::new("import './linked.js'; export function main(){return 1;}");
    project.write("actual.js", "export const x=1;");
    symlink(project.0.join("actual.js"), project.0.join("linked.js")).unwrap();
    failure(&project, "module_nonlocal");
    project.write("lib/value.js", "export const x=1;");
    symlink(project.0.join("lib"), project.0.join("linked-dir")).unwrap();
    project.write(
        "main.mjs",
        "import './linked-dir/value.js'; export function main(){return 1;}",
    );
    failure(&project, "module_nonlocal");
}
