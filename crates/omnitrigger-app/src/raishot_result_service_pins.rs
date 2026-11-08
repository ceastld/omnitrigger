// 文件结果工厂与普通入口按字节校验；SDK源摘要用于诊断版本。
const SDK_SOURCES: &[(&str, &[u8])] = &[
    (
        "crates/omnitrigger-actions/runtime/desktop-api.js",
        include_bytes!("../../../crates/omnitrigger-actions/runtime/desktop-api.js"),
    ),
    (
        "crates/omnitrigger-actions/runtime/host.mjs",
        include_bytes!("../../../crates/omnitrigger-actions/runtime/host.mjs"),
    ),
    (
        "tools/js-engine-probe/src/sdk.js",
        include_bytes!("../../../tools/js-engine-probe/src/sdk.js"),
    ),
];
const RESULT_FACTORY: &[u8] = include_bytes!("raishot_result_factory_pin.js");
const RESULT_FACTORY_SHA: &str = "756a9b7a130c32710b2a959b69e513ec68f23192b2e8aa79ba03c5ef818b47be";
const ENTRY_LENGTH: usize = 652;
const ENTRY_SHA: &str = "887057d96d73afc38deca33f09d8d00a7d8fdb7e732554591009b988ebe903f2";
const PNG_LENGTH: usize = 70;
const PNG_SHA: &str = "ed7a400fdfc370c4ff8f9bffe2a13a5072655d0966a0c891aa5e930c090a9837";
const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 16, 84, 50, 254, 15, 0, 2,
    20, 1, 102, 4, 224, 222, 127, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];
