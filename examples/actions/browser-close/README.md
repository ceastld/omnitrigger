# 关闭指定浏览器标签

此 Node 动作固定所选 `profile`、`windowId` 和 `tabId`，先检查窗口和标签位置，再只派发一次 `tabs.remove(tabId)`。支持普通和 popup 浏览器窗口，不选择活动标签，不依赖 URL 或标题，不发送快捷键。

将完整目录放入配置同级 `actions/browser-close` 后由动作库加载。动作身份 `examples.browser-close/close-tab@1`；`call.json` 的示例身份必须替换。需要 Node 和已安装、连接且允许相关 API 的浏览器连接器。

输入必须是对象，只允许四个字段：必填 `profile` 为 `profiles()` 中用户选定的 `browser_instance_id`；必填 `windowId` 和 `tabId` 为非负安全整数；可选 `timeoutMs` 为 1–60000 整数，默认单次 API 期限 10000ms，并继续受宿主总期限限制。数字 0 是明确 ID，不表示当前目标。ID 只适用于当前浏览器会话。

回执 `status:completed` 和 `closeAcknowledged:true` 表示 API 调用确认成功，`target` 记录所核验的请求目标。它不证明页面已保存、窗口最终数量或后续业务效果。最后一个标签关闭可能使整个窗口退出。前置查询或校验失败直接抛出，副作用不会派发。

关闭调用失败后输出保留 `failed`（明确未开始）、`partial`（宿主报告部分效果）或 `unknown`（超时、断连等未确认结果），同时保留固定错误码、API 方法、效果状态及权限名称。不会保存 URL、标题、错误消息或完整 API 响应，不自动重试、恢复会话或关闭其他标签。

检查和关闭不是原子事务：标签可能在检查后移动或自行关闭。本动作固定原 tab ID，不回退到当时的活动标签，也不能消除浏览器竞争。外层宿主强停可能阻止业务回执交付，不能据此认定没有效果。

没有新增依赖。此业务是明确 ID 的 API 关闭能力；原键盘动作仍由键盘配方保留，不将 API 调用当成 Ctrl+W 的来源等价实现。


本包带当前 SDK 声明副本与 strict 类型检查：仓库根运行 `node tools/script-sdk-check/node_modules/typescript/lib/tsc.js -p examples/actions/browser-close/jsconfig.json`。纯假桥接契约测试为 `node --test tests/browser_close.test.mjs`。
