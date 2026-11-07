# 常用浏览器工作流

本动作包用已有 `ctx.browser.profiles()` 和 `ctx.browser.call()` 组合五个 Node 动作和两个同步轻量入口。Node 入口需要 Node，同步入口需要已安装的 Boa 或 QuickJS 宿主；两者都需要已安装并连接的浏览器连接器，以及 Chrome 对应权限。将整个目录放入所选配置同级 `actions/browser-workflows`，重新加载应用后可在动作库设置触发。开发入口执行 `omnitrigger.exe --run-action <调用 JSON 路径>`。先修改 `call.json` 内的示例配置标识和窗口 ID；其中占位值不能直接使用。

本包没有新增依赖。离线检查在仓库根运行：

```powershell
node --test tests/browser_workflows.test.mjs
```

## 动作和输入

所有动作接受可选 `profile` 和 `timeoutMs`（单次调用 1–60000 毫秒，默认 10000；同时受宿主总期限约束）。`profile` 是 `profiles()` 返回的 `browser_instance_id`，不是名称或配置路径。省略仅允许恰好一个连接；多个连接报 `ambiguous_profile`，指定配置消失报 `profile_unavailable`。解析后每一步都固定使用该标识，不重新选取第一个连接。

| 动作 ID | 必填输入 | 可选输入 | 行为 |
| --- | --- | --- | --- |
| `create-default-tab` | `profile`、`windowId` | `activate`、`focusWindow` | 在指定普通窗口中新建浏览器默认标签页，不传入 URL |
| `create-default-tab-script` | `profile`、`windowId` | `activate`、`focusWindow` | 同一完整业务的同步轻量入口，无需 Node |
| `open-url` | `windowId`、`url` | `tabId`、`activate`、`focusWindow` | 在指定普通窗口中按网址复用；无匹配才新建 |
| `duplicate-tab` | `windowId`、`tabId` | `focusWindow` | 复查源标签仍在指定窗口，再按明确 ID 复制 |
| `restore-session` | `sessionId`、`sessionKind` | 无 | 从最近关闭列表核对 ID 和类型，再恢复该会话 |
| `restore-session-script` | `sessionId`、`sessionKind` | 无 | 同一恢复业务的同步轻量入口，无需 Node |
| `open-bookmark` | `windowId`、`query` | `bookmarkId`、`tabId`、`activate`、`focusWindow` | 搜索书签，得到唯一网页 URL 后复用相同打开流程 |

`windowId` 与 `tabId` 必须是0–2147483647的非负 int32 整数；返回的新标签或窗口身份也按该范围核验。未知字段会报错，避免拼错字段悄悄改变目标。目标 ID 需要在当前浏览器会话中获取，不能把它当作跨重启稳定标识；绑定中保存过的 ID 可能失效。

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

## 新建默认标签页

`create-default-tab` 要求明确的 `profile` 和普通窗口 `windowId`，不接受 `url` 或 `tabId`。它核验窗口后只调用一次 `tabs.create`，省略 URL，由浏览器决定默认页。默认激活新标签，只有显式 `focusWindow:true` 才请求窗口聚焦；回执中的 `addressBarRequested:false` 表示没有发送地址栏快捷键。

两种默认新标签入口要求创建回执在请求激活时确认 `active:true`；未确认时保留 `partial`、已知标签和创建步骤，并停止后续聚焦。显式聚焦要求回执返回指定窗口 ID 与 `focused:true`；身份或聚焦未确认时保留 `partial`，不重试。

新建、窗口聚焦和地址栏获得输入焦点是不同结果。此动作不复用已有标签，不选择当前前台，也不保证地址栏焦点。失败沿用下文的部分成功回执，不重试创建。

## 打开网址和书签

URL 仅支持不含用户名、密码的完整 HTTP/HTTPS 地址，用标准 `URL.href` 规范化后精确比较，包含路径、查询和 fragment；不按域名、前缀或通配符匹配。`pendingUrl` 优先于已提交 URL，避免复用正在离开目标页面的标签。多个匹配报 `ambiguous_tab`，可显式传入匹配的 `tabId`；显式标签不匹配时停止，不新建替代标签。浏览器不能提供完整标签网址时报告 `permission_required` 和 `requiredPermissions: ['tabs']`，需要扩展有 `tabs` 或足以覆盖相关 URL 的站点授权。

默认 `activate: true` 会激活目标窗口内的标签。`open-url` 和 `open-bookmark` 新建或复用标签且要求激活时，均要求相应回执确认 `active: true`；复用标签还核对 `tabs.update` 返回的标签和窗口 ID；成功调用没有返回值或未确认激活时报告 `partial` / `activation_unconfirmed`，身份异常也保留 `partial` 和已确认步骤，停止后续窗口聚焦且不重试。这两个动作及 `duplicate-tab` 的聚焦调用需返回指定窗口 ID 与 `focused: true`，否则保留 `partial` 和已知标签，不重试；此字段仍不证明 Windows 原目标或地址栏焦点。`focusWindow` 默认 false，不主动把浏览器窗口带到前台；确实希望定位到该窗口时设 true。`activate: false` 时复用没有副作用，新建使用后台标签，且不能同时要求 `focusWindow: true`。这些选项仅适用于 `open-url`、`open-bookmark` 和 `create-default-tab`；复制、恢复的激活行为由 Chrome API 决定。

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

`restore-session` 与 `restore-session-script` 共用同一组有序步骤和回执规则；同步入口导出 `main(ctx)`，使用现有 `ScriptContext` 直接取得同步返回值。`restore-session-call.json` 与 `restore-session-script-call.json` 分别提供调用模板。`sessionId` 必须来自所选 profile 的最近关闭列表，不是连接器的 `session_id`，也不是标签 ID。两入口均不请求权限、不重试恢复；已经确认恢复但返回身份缺失或超出上述范围时保留 `partial` 和已确认步骤。

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


## 同步轻量默认新标签

安装既有 Boa 或 QuickJS 轻量宿主后，可使用新增 `create-default-tab-script`（runtime 为 `script`、具名同步 `main(ctx)`），不需要 Node。原 `create-default-tab` 的 Node 入口与动作版本1保持原身份，两种入口共用同一组默认新标签业务步骤。

`default-new-tab-script-call.json` 提供调用模板，需要明确替换 profile/windowId；默认后台示例使用 `activate:false`、`focusWindow:false`。参数校验、首次API调用前的输入快照、profile/普通窗口检查、默认页省略url、创建回执及partial/unknown语义与Node入口一致。创建或聚焦的派发前取消按宿主effectState处理；已经确认创建后再取消或聚焦失败会保留partial及明确目标，不自动重试或补偿关闭标签。

同步入口返回JSON，不返回Promise；宿主进程被外层取消或强停时，仍由既有ActionService管理终态，不能保证脚本有机会返回回执。脚本内部不查询前台、剪贴板、URL或活动标签。


本包的 `jsconfig.json` 同时检查五个 Node、两个同步入口及共用模块。Node 上下文引用同包 `sdk/api.d.ts`，同步入口引用 `sdk/script-types.d.ts`；移动工程时保留整个 `sdk` 目录。这里沿用包内声明，不依赖仓库其它目录，也不因类型检查自动升级宿主或扩展。

仓库安装现有 SDK 开发依赖后，可运行统一静态检查：

```powershell
node tools/script-sdk-check/check-project-types.mjs examples/actions/browser-workflows
```

检查不导入或运行作者入口，不连接浏览器。通过只表示当前声明和静态入口约束没有错误，仍需分别核对运行时能力、扩展权限及目标浏览器行为。配置中的 DOM 类型用于 `URL` 等标准声明，不表示同步宿主提供任意 DOM API。
