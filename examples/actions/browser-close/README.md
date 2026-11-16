# 关闭指定浏览器标签

本包 Node 与同步轻量入口固定所选 `profile`、`windowId` 和 `tabId`，先检查窗口和标签位置，再只派发一次 `tabs.remove(tabId)`。支持普通和 popup 浏览器窗口，不选择活动标签，不依赖 URL 或标题，不发送快捷键。

将完整目录放入配置同级 `actions/browser-close` 后由动作库加载。原 Node 动作身份为 `examples.browser-close/close-tab@1`，使用 `call.json`。同步轻量动作是 `examples.browser-close/close-tab-script@1`，使用 `close-tab-script-call.json`，无需 Node，需已安装的 Boa 或 QuickJS 同步宿主。两个模板的示例身份都必须替换，并需要已安装、连接且允许相关 API 的浏览器连接器。

输入必须是对象，只允许四个字段：必填 `profile` 为 `profiles()` 中用户选定的 `browser_instance_id`；必填 `windowId` 和 `tabId` 为 0–2147483647 的整数（Chrome API 的非负 int32 ID，不截断或强制转换）；可选 `timeoutMs` 为 1–60000 整数，默认单次 API 期限 10000ms，并继续受宿主总期限限制。数字 0 是明确 ID，不表示当前目标。ID 只适用于当前浏览器会话。

回执 `status:completed` 和 `closeAcknowledged:true` 表示 API 调用确认成功，`target` 记录所核验的请求目标。它不证明页面已保存、窗口最终数量或后续业务效果。最后一个标签关闭可能使整个窗口退出。前置查询或校验失败直接抛出，副作用不会派发。

关闭调用失败后输出保留 `failed`（明确未开始）、`partial`（宿主报告部分效果）或 `unknown`（超时、断连等未确认结果），同时保留固定错误码、API 方法、效果状态及权限名称。不会保存 URL、标题、错误消息或完整 API 响应，不自动重试、恢复会话或关闭其他标签。

检查和关闭不是原子事务：标签可能在检查后移动或自行关闭。本动作固定原 tab ID，不回退到当时的活动标签，也不能消除浏览器竞争。外层宿主强停可能阻止业务回执交付，不能据此认定没有效果。

没有新增依赖。此业务是明确 ID 的 API 关闭能力；原键盘动作仍由键盘配方保留，不将 API 调用当成 Ctrl+W 的来源等价实现。


本包带当前 SDK 声明副本与 strict 类型检查：仓库根运行 `node tools/script-sdk-check/node_modules/typescript/lib/tsc.js -p examples/actions/browser-close/jsconfig.json`。纯假桥接契约测试为 `node --test tests/browser_close.test.mjs`。


同步入口是具名 `main(ctx)`，返回JSON回执，不返回Promise。两个入口使用同一参数、身份和回执校验，分别直接调用对应的异步/同步API；正常路径固定为profiles、windows.get、tabs.get、tabs.remove。宿主整体强停仍可能阻止回执交付，不能据此认定没有效果。

本地 `jsconfig.json` 同时严格检查两个入口、共享纯helper和随包的同步SDK声明。此声明来自当前正式SDK；没有给同步作者注入Promise版browser接口。入口契约测试：`node --test tests/browser_close_sync.test.mjs`；类型正反例：`node tools/script-sdk-check/node_modules/typescript/lib/tsc.js -p tests/browser_close_sync_types.json`。

## 普通作者工程检查

在本目录运行 `npm ci --ignore-scripts` 后，执行 `npm run check`。检查使用固定TypeScript 5.9.3和本包SDK，严格检查两个已有入口、共享算法及Node默认导出，不执行关闭动作或连接浏览器。

本例仅依赖ES2022和显式宿主SDK，`types:[]`；不向Node或同步Script入口添加window、document等DOM全局。Node入口通过JSDoc的ActionContext使用Promise，Script入口通过ScriptMain使用同步回执。当前代码不依赖Node原生全局，若作者增加此类依赖，应为相应入口单独配置显式环境，避免泄漏给同步入口。

仓库内还可运行 `node tools/script-sdk-check/check-project-types.mjs examples/actions/browser-close`。声明可用既有 `tools/script-project.mjs refresh --runtime node --project <本目录>` 刷新整组公共SDK；刷新保留入口、清单、配置、说明及开发依赖。旧包中未经识别的声明仍由工具拒绝，不自动覆盖作者修改。
