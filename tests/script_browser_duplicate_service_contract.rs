// 作为PR8 service_contract子模块，仅复用既有普通Service/Runtime/BrowserHost/broker设施。
use super::super::DuplicateScenario;
use super::{Fixture, Mode, Package, invoke_prepared};
use omnitrigger_actions::{ActionCall, NativeHost};
use serde_json::json;
use std::{sync::atomic::Ordering, time::Duration};

const AUTHOR_FILES: &[(&str, &[u8])] = &[
    (
        "action.json",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/browser-workflows/action.json"
        )),
    ),
    (
        "workflows.mjs",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/browser-workflows/workflows.mjs"
        )),
    ),
    (
        "duplicate-tab.mjs",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/browser-workflows/duplicate-tab.mjs"
        )),
    ),
    (
        "duplicate-tab-script.mjs",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/browser-workflows/duplicate-tab-script.mjs"
        )),
    ),
    (
        "default-new-tab.mjs",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/browser-workflows/default-new-tab.mjs"
        )),
    ),
    (
        "default-new-tab-script.mjs",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/browser-workflows/default-new-tab-script.mjs"
        )),
    ),
    (
        "open-url.mjs",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/browser-workflows/open-url.mjs"
        )),
    ),
    (
        "open-bookmark.mjs",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/browser-workflows/open-bookmark.mjs"
        )),
    ),
    (
        "restore-session.mjs",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/browser-workflows/restore-session.mjs"
        )),
    ),
    (
        "restore-session-script.mjs",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/browser-workflows/restore-session-script.mjs"
        )),
    ),
];

#[test]
#[ignore = "真实普通复制作者+正常Node/QuickJS/Boa；复用PR8自有broker，不连接Chrome"]
fn duplicate_author_service_contract() {
    let mode = Mode::selected();
    for scenario in [
        DuplicateScenario::Distinct,
        DuplicateScenario::SameSource,
        DuplicateScenario::FocusUnknown,
    ] {
        let mut fixture = Fixture::with_duplicate(mode.wait() + Duration::from_secs(8), scenario);
        let package = Package::with_files(&fixture, mode, AUTHOR_FILES);
        let call = ActionCall {
            package_id: "examples.browser-workflows".into(),
            action_id: if mode == Mode::Node {
                "duplicate-tab"
            } else {
                "duplicate-tab-script"
            }
            .into(),
            action_version: 1,
            input: json!({"profile":"work","windowId":4,"tabId":7,"focusWindow":true,"timeoutMs":1000}),
        };
        let (result, host) = invoke_prepared(&fixture, mode, package, call);
        assert!(!host.native_context_available());
        let output = result.expect("普通Service应交付工作流业务JSON；不能把宿主成功当业务完成");
        let duplicate_step = json!({"method":"tabs.duplicate","sourceTabId":7,"windowId":4,"tabId":if scenario==DuplicateScenario::SameSource {7} else {9}});
        let expected = match scenario {
            DuplicateScenario::Distinct => {
                json!({"status":"completed","profile":"work","operation":"duplicate-tab","steps":[duplicate_step,{"method":"windows.update","windowId":4}],"target":{"tabId":9,"windowId":4}})
            }
            DuplicateScenario::SameSource => {
                json!({"status":"partial","profile":"work","operation":"duplicate-tab","steps":[duplicate_step],"error":{"code":"invalid_response","method":"tabs.duplicate","effect_state":"partial"}})
            }
            DuplicateScenario::FocusUnknown => {
                json!({"status":"partial","profile":"work","operation":"duplicate-tab","steps":[duplicate_step],"target":{"tabId":9,"windowId":4},"error":{"code":"result_unknown","method":"windows.update","effect_state":"unknown"}})
            }
        };
        assert_eq!(output, expected);
        let mut expected_requests = vec![("tabs.get", json!([7])), ("tabs.duplicate", json!([7]))];
        if scenario != DuplicateScenario::SameSource {
            expected_requests.push(("windows.update", json!([4,{"focused":true}])));
        }
        assert_eq!(
            fixture.calls.load(Ordering::SeqCst),
            expected_requests.len()
        );
        let requests = fixture.requests.lock().unwrap();
        assert_eq!(requests.len(), expected_requests.len());
        for (request, (method, args)) in requests.iter().zip(&expected_requests) {
            assert_eq!(request["method"], *method);
            assert_eq!(&request["args"], args);
            assert_eq!(request["browser_instance_id"], "work");
            assert_eq!(request["session_id"], "s-work");
            assert_eq!(request["timeout_ms"], 1000);
        }
        assert_eq!(
            requests
                .iter()
                .filter(|r| r["method"] == "tabs.duplicate")
                .count(),
            1
        );
        assert_eq!(
            requests
                .iter()
                .filter(|r| r["method"] == "windows.update")
                .count(),
            usize::from(scenario != DuplicateScenario::SameSource)
        );
        drop(requests);
        let calls = host.calls.lock().unwrap();
        if mode == Mode::Node {
            assert!(calls.is_empty(), "Node专用SDK浏览器链不得改走Native");
        } else {
            assert_eq!(calls.len(), expected_requests.len() + 1);
            assert_eq!(calls[0].0, "browser.profiles");
            assert_eq!(calls[0].1, json!({"options":{"timeoutMs":1000}}));
            for ((method, params), (api, args)) in calls.iter().skip(1).zip(&expected_requests) {
                assert_eq!(method, "browser.call");
                assert_eq!(
                    params,
                    &json!({"method":api,"args":args,"options":{"profile":"work","timeoutMs":1000}})
                );
            }
        }
        assert!(
            calls
                .iter()
                .all(|(method, _)| method == "browser.profiles" || method == "browser.call")
        );
        drop(calls);
        eprintln!(
            "普通复制作者Service场景 mode={mode:?} scenario={scenario:?} 精确回执/单次派发已核对"
        );
        fixture.finish();
    }
}
