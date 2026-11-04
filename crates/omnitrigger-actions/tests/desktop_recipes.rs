//! 示例通过真实轻量 JS 宿主执行；所有桌面调用由内存宿主接收。
use omnitrigger_actions::{
    ActionCall, ActionError, ActionRuntime, CancellationToken, InvocationContext, NativeHost,
    PackageCatalog, ResolvedAction, ScriptHostRuntime,
};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Mutex};

struct Host {
    process: &'static str,
    selection: &'static str,
    fail_method: Option<&'static str>,
    calls: Mutex<Vec<(String, Value)>>,
}

impl Host {
    fn new(process: &'static str) -> Self {
        Self {
            process,
            selection: "",
            fail_method: None,
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl NativeHost for Host {
    fn call(
        &self,
        method: &str,
        params: Value,
        _: &CancellationToken,
    ) -> Result<Value, ActionError> {
        self.calls.lock().unwrap().push((method.into(), params));
        if self.fail_method == Some(method) {
            let mut error = ActionError::new("target_changed", "原目标已变化。");
            error.effect_state = Some("not_started".into());
            return Err(error);
        }
        Ok(match method {
            "target.info" => json!({"processName": self.process}),
            "selection.copy" => json!(self.selection),
            _ => json!({"submitted": true}),
        })
    }
}

fn action(package: &str, id: &str, input: Value) -> ResolvedAction {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/actions");
    PackageCatalog::load(&root)
        .resolve(&ActionCall {
            package_id: package.into(),
            action_id: id.into(),
            action_version: 1,
            input,
        })
        .unwrap()
}

fn execute(action: &ResolvedAction, host: &Host) -> Result<Value, ActionError> {
    let runtime = ScriptHostRuntime::new(PathBuf::from(
        std::env::var_os("OMNITRIGGER_TEST_SCRIPT_HOST").expect("设置实际轻量 JS 宿主路径"),
    ));
    runtime.execute_with_host(
        action,
        &InvocationContext {
            invocation_id: 1,
            binding_id: 2,
        },
        &CancellationToken::new(),
        Some(host),
    )
}

#[test]
fn desktop_recipes_resolve_standard_script_entries() {
    for (package, id, entry) in [
        ("examples.common-desktop", "close-by-process", "close.js"),
        ("examples.common-desktop", "window-action", "window.js"),
        ("examples.common-search", "web-search", "search.js"),
    ] {
        let resolved = action(package, id, json!({}));
        assert_eq!(resolved.definition.runtime, "script");
        assert_eq!(resolved.definition.entry, entry);
        assert!(resolved.entry_path.is_file());
    }
}

#[test]
#[ignore = "需要显式指定实际构建的 Boa 或 QuickJS 宿主；仅内存桌面"]
fn desktop_recipes_close_keeps_process_branch_and_timing() {
    let action = action(
        "examples.common-desktop",
        "close-by-process",
        json!({"ctrlWProcesses": ["Acrobat"]}),
    );
    for (process, operation, modifiers, keys, delay) in [
        ("Acrobat.exe", "ctrl_w", vec![162], vec![87], 0),
        ("acrobat.exe", "alt_f4", vec![164], vec![115], 0),
        ("chrome.exe", "alt_f4", vec![164], vec![115], 0),
        ("wps.exe", "alt_f4", vec![164], vec![115], 10),
    ] {
        let host = Host::new(process);
        assert_eq!(
            execute(&action, &host).unwrap(),
            json!({"processName": process, "operation": operation, "submitted": true})
        );
        assert_eq!(
            *host.calls.lock().unwrap(),
            vec![
                ("target.info".into(), json!({})),
                (
                    "keyboard.sequence".into(),
                    json!({"modifiers": modifiers, "keys": keys, "key_down_delay_ms": delay})
                )
            ]
        );
    }
}

#[test]
#[ignore = "需要显式指定实际构建的 Boa 或 QuickJS 宿主；仅内存桌面"]
fn desktop_recipes_validate_before_input_and_do_not_retry_after_fault() {
    for input in [
        json!({}),
        json!({"ctrlWProcesses": ["Acrobat.exe"]}),
        json!({"ctrlWProcesses": ["Acrobat"], "keyDownDelayMs": 1001}),
    ] {
        let host = Host::new("Acrobat.exe");
        let error = execute(
            &action("examples.common-desktop", "close-by-process", input),
            &host,
        )
        .unwrap_err();
        assert_eq!(error.code, "invalid_input");
        assert!(host.calls.lock().unwrap().is_empty());
    }
    let close = action(
        "examples.common-desktop",
        "close-by-process",
        json!({"ctrlWProcesses": ["Acrobat"]}),
    );
    for (method, count) in [("target.info", 1), ("keyboard.sequence", 2)] {
        let mut host = Host::new("Acrobat.exe");
        host.fail_method = Some(method);
        let error = execute(&close, &host).unwrap_err();
        assert_eq!(error.code, "target_changed");
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        assert_eq!(host.calls.lock().unwrap().len(), count);
    }
}

#[test]
#[ignore = "需要显式指定实际构建的 Boa 或 QuickJS 宿主；仅内存桌面"]
fn desktop_recipes_window_operations_have_explicit_receipts() {
    for operation in ["toggle_maximize", "minimize", "close"] {
        let host = Host::new("Code.exe");
        let request = action(
            "examples.common-desktop",
            "window-action",
            json!({"operation": operation}),
        );
        assert_eq!(
            execute(&request, &host).unwrap(),
            json!({"operation": operation, "submitted": true})
        );
        assert_eq!(
            *host.calls.lock().unwrap(),
            vec![("window.action".into(), json!({"operation": operation}))]
        );
    }
}

#[test]
#[ignore = "需要显式指定实际构建的 Boa 或 QuickJS 宿主；仅内存桌面"]
fn desktop_recipes_search_does_not_copy_for_invalid_parameters_or_open_empty_selection() {
    // 成功打开 URL 不通过 NativeHost 路由；此测试只覆盖不会调用 ctx.open 的路径。
    for input in [
        json!({"source": "selection", "urlTemplate": "file:///example/%s"}),
        json!({"source": "selection", "maxCodePoints": 0}),
        json!({"source": "text", "text": ""}),
    ] {
        let host = Host::new("Code.exe");
        let error = execute(
            &action("examples.common-search", "web-search", input),
            &host,
        )
        .unwrap_err();
        assert_eq!(error.code, "invalid_input");
        assert!(host.calls.lock().unwrap().is_empty());
    }
    let host = Host::new("Code.exe");
    let error = execute(
        &action(
            "examples.common-search",
            "web-search",
            json!({"source": "selection"}),
        ),
        &host,
    )
    .unwrap_err();
    assert_eq!(error.code, "invalid_selection");
    assert_eq!(error.effect_state.as_deref(), Some("unknown"));
    assert_eq!(
        *host.calls.lock().unwrap(),
        vec![("selection.copy".into(), json!({}))]
    );
}
