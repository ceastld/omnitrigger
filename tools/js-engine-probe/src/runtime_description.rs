//! 安装与环境检查使用的有界自述；接口存在不代表应用桥接和权限已经可用。
use serde_json::{json, Value};

pub fn describe() -> Value {
    let mut capabilities = vec![
        "actions.stop_other_instances.v1",
        "clipboard.history.v1",
        "clipboard.read_data.v1",
        "desktop.copy_selection_data.v1",
    ];
    // 非 Windows 的采集分支明确返回 unsupported_platform，不能声明可执行支持。
    if cfg!(windows) {
        capabilities.push("process.capture.v1");
        capabilities.push("system.open_file_in_parent.v1");
    }
    json!({
        "schema_version":1,
        "product":"omnitrigger-script-host",
        "version":env!("CARGO_PKG_VERSION"),
        "engine":if cfg!(feature="quickjs") { "quickjs" } else { "boa" },
        "api_version":1,
        "protocol_schema":1,
        "capabilities":capabilities,
    })
}
