# 常用浏览器工作流

本动作包用已有 `ctx.browser.profiles()` 和 `ctx.browser.call()` 组合四个 Node 动作。需要 Node、已安装并连接的浏览器连接器，以及 Chrome 对应权限。将整个目录放入所选配置同级 `actions/browser-workflows`，重新加载应用后可在动作库设置触发。开发入口执行 `omnitrigger.exe --run-action <调用 JSON 路径>`。先修改 `call.json` 内的示例配置标识和窗口 ID；其中占位值不能直接使用。

本包没有新增依赖。离线检查在仓库根运行：

```powershell
node --test tests/browser_workflows.test.mjs
```

## 动作和输入

所有动作接受可选 `profile` 和 `timeoutMs`（单次调用 1–60000 毫秒，默认 10000；同时受宿主总期限约束）。`profile` 是 `profiles()` 返回的 `browser_instance_id`，不是名称或配置路径。省略仅允许恰好一个连接；多个连接报 `ambiguous_profile`，指定配置消失报 `profile_unavailable`。解析后每一步都固定使用该标识，不重新选取第一个连接。

| 动作 ID | 必填输入 | 可选输入 | 行为 |
| --- | --- | --- | --- |
| `open-url` | `windowId`、`url` | `tabId`、`activate`、`focusWindow` | 在指定普通窗口中按网址复用；无匹配才新建 |
| `duplicate-tab` | `windowId`、`tabId` | `focusWindow` | 复查源标签仍在指定窗口，再按明确 ID 复制 |
| `restore-session` | `sessionId`、`sessionKind` | 无 | 从最近关闭列表核对 ID 和类型，再恢复该会话 |
| `open-bookmark` | `windowId`、`query` | `bookmarkId`、`tabId`、`activate`、`focusWindow` | 搜索书签，得到唯一网页 URL 后复用相同打开流程 |

`windowId` 与 `tabId` 必须是非负安全整数。未知字段会报错，避免拼错字段悄悄改变目标。目标 ID 需要在当前浏览器会话中获取，不能把它当作跨重启稳定标识；绑定中保存过的 ID 可能失效。

在自己的 Node 动作中可通过现有 API 查询目标，随后把选定的明确 ID 传入本包。多配置时先由用户选择配置，再读取它的窗口与标签，不把最近活动窗口解释为动作原目标：

```js
const profiles = await ctx.browser.profiles();
// profile 必须取自用户选定的 profiles 项。
const windows = await ctx.browser.call('windows.getAll', [{ windowTypes: ['normal'] }], { profile });
const tabs = await ctx.browser.call('tabs.query', [{ windowId }], { profile });
const bookmarks = await ctx.browser.call('bookmarks.search', [{ query: '文档' }], { profile });
const closed = await ctx.browser.call('sessions.getRecentlyClosed', [{ maxResults: 25 }], { profile });
```

这些结果只应在脚本中筛选或提供给自己的选择界面；不要直接把整棵书签树或所有标签内容写入动作历史。本包输出只保留 ID、步骤和错误码，不包含网页 URL、标题和搜索词。动作输入本身仍遵循应用现有保存规则，应避免在绑定 URL 中放置秘密。

## 打开网址和书签

URL 仅支持不含用户名、密码的完整 HTTP/HTTPS 地址，用标准 `URL.href` 规范化后精确比较，包含路径、查询和 fragment；不按域名、前缀或通配符匹配。`pendingUrl` 优先于已提交 URL，避免复用正在离开目标页面的标签。多个匹配报 `ambiguous_tab`，可显式传入匹配的 `tabId`；显式标签不匹配时停止，不新建替代标签。浏览器不能提供完整标签网址时报告 `permission_required` 和 `requiredPermissions: ['tabs']`，需要扩展有 `tabs` 或足以覆盖相关 URL 的站点授权。

默认 `activate: true` 会激活目标窗口内的标签。`focusWindow` 默认 false，不主动把浏览器窗口带到前台；确实希望定位到该窗口时设 true。`activate: false` 时复用没有副作用，新建使用后台标签，且不能同时要求 `focusWindow: true`。这些选项仅适用于 `open-url` 和 `open-bookmark`；复制、恢复的激活行为由 Chrome API 决定。

书签搜索先排除文件夹。多个网页书签匹配报 `ambiguous_bookmark`，使用先前搜索得到的 `bookmarkId` 消歧；指定 ID 必须仍出现在本次搜索中。非网页 URL（如 bookmarklet）报 `unsupported_url`。

例如 `open-bookmark` 的调用内容：

```json
{
  "package_id": "examples.browser-workflows",
  "action_id": "open-bookmark",
  "action_version": 1,
  "input": {
    "profile": "选定的 browser_instance_id",
    "windowId": 123,
    "query": "文档",
    "bookmarkId": "选定的书签 ID",
    "activate": true,
    "focusWindow": true
  }
}
```

## 恢复与复制

恢复需要 `sessionKind: "tab"` 或 `"window"`，以及该类型的 `sessionId`。只在 `sessions.getRecentlyClosed({ maxResults: 25 })` 中找到唯一同类型会话时执行 `sessions.restore(sessionId)`。会话过期、类型不符或已不在该列表时报 `session_unavailable`；不会回退恢复最近关闭项，也不读取同步设备的会话。恢复位置由 Chrome 决定，不能要求恢复到另一个指定窗口。

复制先执行 `tabs.get(tabId)` 并核对窗口，再执行一次 `tabs.duplicate(tabId)`。Chrome 可以返回没有副本身份的成功回执，本包此时报告 `partial`，不会查询活动标签猜测副本或再次复制。

## 回执、权限和部分成功

副作用开始前的错误会抛出，保留 `code`、`method`、`effectState`、`requiredPermissions`。授权不足时由用户通过扩展支持的方式授权；本包不修改设置、不自动请求权限。已安装连接器未暴露相关 API 或权限时会保留其错误，不能用仓库源码中的能力推断已安装版本可用。

一旦尝试副作用，返回结构化业务回执，保留已确认步骤和已知目标身份。调用方必须检查 **输出的 `status`**：宿主成功返回 JSON 不等于工作流全部完成。

| 输出 `status` | 含义 |
| --- | --- |
| `completed` | 所有请求步骤已确认完成（不包含页面载入或业务页面验收） |
| `partial` | 至少一个副作用已确认；后续失败，或缺少可核对的结果身份 |
| `failed` | 首个副作用明确未开始 |
| `unknown` | 首个副作用已经尝试，但结果未确认 |

例如新建完成而窗口聚焦失败：

```json
{
  "status": "partial",
  "profile": "work",
  "operation": "open-url",
  "steps": [{ "method": "tabs.create", "windowId": 4, "tabId": 9 }],
  "disposition": "created",
  "target": { "tabId": 9, "windowId": 4 },
  "error": { "code": "permission_required", "method": "windows.update", "effect_state": "partial" }
}
```

`error.effect_state` 描述失败步骤或已确认前序效果，`unknown` 不会覆盖 `steps` 中已确认的信息。不会自动重试、回滚、关闭已创建标签。宿主整体取消或期限到达时，宿主可能先返回取消/超时错误而无法交付脚本回执，不能据此声称没有副作用。

查询、核对和修改不是 Chrome 原子事务：复查可发现期间已发生的变化，但不能消除紧接着的移动、导航或会话变化。源标签或窗口发生变化后停止，不用最活跃标签代替。

API 语义依据：[Chrome Tabs API](https://developer.chrome.com/docs/extensions/reference/api/tabs)、[Windows API](https://developer.chrome.com/docs/extensions/reference/api/windows)、[Sessions API](https://developer.chrome.com/docs/extensions/reference/api/sessions)。
