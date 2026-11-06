# 指定标签弹出窗口

本包的 Node 与同步 Script 动作使用已有 `ctx.browser.profiles()` / `ctx.browser.call()`，将选定普通窗口中的一个明确标签移入新建的浏览器 `popup` 窗口。整个目录放入配置同级 `actions/browser-popout` 后可由动作库加载；需要已连接的浏览器连接器。Node 动作需要 Node；同步 Script 动作需要已就绪的 Script 运行时，入口导出同步 `main(ctx)`，浏览器调用直接返回值。`call.json` 与 `pop-out-tab-script-call.json` 分别是两种运行时的调用模板，必须替换示例 profile、窗口和标签 ID。

Node 动作身份为 `examples.browser-popout/pop-out-tab@1`，同步动作身份为 `examples.browser-popout/pop-out-tab-script@1`，分别单实例执行。两者使用相同输入、API 顺序和业务回执。输入字段如下，其他字段会报 `invalid_input`：

| 字段 | 要求 |
| --- | --- |
| `profile` | 必填，来自所选连接的 `browser_instance_id` |
| `windowId` | 必填，普通源窗口的0–2147483647 整数 ID（非负 int32） |
| `tabId` | 必填，该窗口内明确标签的0–2147483647 整数 ID（非负 int32） |
| `focusWindow` | 可选布尔值，默认 false；true 请求新窗口获得焦点 |
| `timeoutMs` | 可选整数，单次 API 期限 1–60000 毫秒，默认 10000；仍受宿主总期限限制 |

数字 0 按明确 ID 核验，没有“当前标签”含义。ID 属于当前浏览器会话，不应作为跨重启稳定身份保存。可先用既有 API 获取用户选定的 profile、窗口和标签；动作不自动查询前台或活动标签。

动作先核对连接、普通源窗口及隐私模式，再读取明确 `tabId` 确认其所在窗口。只派发一次 `windows.create({tabId,type:'popup',focused,incognito})`，保留源窗口的隐私模式，不传 URL、不复制标签。创建回执的 `tabs` 可以省略；随后只用 `tabs.get(tabId)` 核验原标签确实位于返回的新窗口。浏览器可能因最后一个标签移走而关闭源窗口。聚焦请求不表示地址栏获得输入焦点。

调用方必须检查业务输出的 `status`：

| 状态 | 含义 |
| --- | --- |
| `completed` | 创建已确认，且按同一 tab ID 复查到新窗口；不证明页面业务状态或焦点 |
| `partial` | API 已确认创建但身份/复查不完整，或宿主明确报告部分效果；只保留已确认的 `createdWindowId` 和步骤 |
| `failed` | 创建请求明确未开始 |
| `unknown` | 已尝试创建，结果未确认 |

所有前置校验或读取错误直接抛出，沿用宿主结构化错误。创建开始后的错误转为业务回执，保留错误码、API 方法、效果状态和权限名称，不保留错误消息、URL、标题或整份 API 响应。输出的 `target` 仅在创建身份和后续标签位置都已核验时提供。`createdWindowId` 只表示创建回执中的已知新窗口 ID，不能单独证明标签仍在其中。

没有自动重试、回滚、关闭窗口或移回源窗口。检查与创建不是原子事务，无法消除检查后立即移动标签等竞争；后置复查只能发现部分变化。宿主取消或强停可能来不及交付脚本回执，不能据此认定没有效果。需要的扩展授权仍由 Chrome 和连接器决定，本动作不申请或修改授权。

此包不新增依赖。API 参数依据 [Chrome Windows API](https://developer.chrome.com/docs/extensions/reference/api/windows#method-create)。

宿主首次创建调用报告 `effectState:partial` 时，业务也保留 `status:partial`；没有成功响应就不会从错误附带的嵌套结果猜测窗口 ID、目标或确认步骤。超时/断连的 unknown 仍是 unknown，不作为可重试或未产生效果的依据。
