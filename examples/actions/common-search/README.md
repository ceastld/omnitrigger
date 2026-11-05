# 文本搜索与本地文件

`web-search` 是可直接编辑的标准同步 JS。`source` 必须明确指定为 `selection` 或 `text`。前者向原目标发送复制命令并读取新文本，复制结果留在剪贴板；后者只使用调用参数 `text`，不读取桌面或剪贴板。

`urlTemplate` 默认是 `https://www.google.com/search?q=%s`。可改成带路径的 HTTP(S) 地址，路径或查询中必须恰好包含一个 `%s`。脚本先检查模板和参数，再复制选区；搜索词用 `encodeURIComponent` 编码，不作为原始 URL 拼接。

`maxCodePoints` 默认 128，范围 1–2048。截断按 Unicode 码点计数，不拆开代理对。文本保留原始空格和换行；纯空白、NUL 和无效 Unicode 会报错。空选区不会回退到旧剪贴板。

`web-search` 只做文本网页搜索。它不会检查文本是否为本地文件，也不自动选择浏览器或复用已有标签页。选区复制的重试和超时由现有宿主 API 决定，脚本不额外覆盖。

`submitted` 表示系统接纳打开 URL 的请求，不能证明网页加载成功。返回值只包含来源、搜索词码点数与是否截断，不返回搜索词或完整 URL。

纯脚本检查不会读取真实选区或打开浏览器：

```powershell
node --test examples/actions/common-search/recipes.test.mjs
```

## 单实例与调用契约

包版本 `1.2.0` 在 `action.json` 中为 `web-search` 和 `open-or-search` 分别显式设置 `single_instance: true`。需要支持此清单字段的动作服务；单实例由服务调度实现，不在 JS 全局变量中加锁，也不依赖两个独立脚本宿主共享内存。

同一个 `ActionService` 内，单实例身份是 `package_id + action_id`：不同绑定、输入、动作版本或运行时都不另分锁。同一动作运行期间，经其他绑定再次提交会返回 `action_busy`，`effect_state: "not_started"`，不会排队、替换、执行或自动重试；重复同一绑定时，既有 `binding_busy` 错误优先。两个不同动作分别占用自己的身份，`web-search` 和 `open-or-search` 不互锁。

取消请求本身不会立即解锁；要等运行时实际结束并完成清理。正常、失败或取消结束后即可再次提交，不必先由界面读取终态；绑定占用与总配额仍按动作服务既有终态读取协议处理。清单重载也不能提前解除正在运行的单实例占用。范围限于同一个动作服务，不保证跨服务或跨进程互斥。

此策略不增加调用参数：两个动作的契约版本仍为 `1`，既有 `source`、`text`、`urlTemplate`、`maxCodePoints` 和返回值保持不变。四份调用示例无需修改；若要调整策略，应编辑动作清单中的布尔字段，不能在 `input` 中加入同名字段来启用或关闭策略。动作服务不支持该字段时，不能依靠脚本自行获得单实例保证。

## 打开本地文件或搜索文本

新增动作 `open-or-search`（契约版本 1，入口 `open-or-search.js`）为标准同步 JS，使用轻量宿主的 `ctx.fs.stat(path)`。需要提供该 API 的应用、SDK 和轻量宿主，旧版宿主没有此接口时返回 `fs_unavailable`，不会改为搜索文件路径。

它接受与 `web-search` 相同的 `source`、`text`、`urlTemplate`、`maxCodePoints`。模板、来源、文本长度、Unicode 和截断规则相同，先完成参数校验再复制。`source: "selection"` 仍向本次动作的原目标复制新选区一次，结果保留在剪贴板；空选区、复制失败、取消均不使用旧剪贴板。`source: "text"` 不访问选区或剪贴板。两个动作不共享运行时状态。

普通文件存在时打开；目录或不存在的本地路径会作为搜索词提交给配置的搜索引擎：

| 输入或元数据结果 | 行为 |
| --- | --- |
| 普通文本 | 按原搜索模板编码、按码点截断后提交网页搜索；不调用 `stat` |
| Windows 普通盘符绝对路径，如 `D:\SyntheticExamples\guide.txt` | 先 `ctx.fs.stat` 原字符串，不截断、不去引号、不展开变量 |
| `kind: "file"` | 一次 `ctx.open` 原路径，不转成 URL，不读文件内容 |
| `kind: "directory"` | 继续原文本搜索流程：编码原路径，应用模板和码点上限，不打开目录 |
| `kind: "missing"` | 继续原文本搜索流程：编码原路径，应用模板和码点上限 |
| `kind: "other"` | `unsupported_path_type`，不打开、不搜索 |
| 畸形元数据回执 | `invalid_response`，不猜成不存在、不搜索 |
| 元数据或打开接口错误 | 原样抛出，不重试、不搜索回退 |

路径意图按明确前缀和形态识别：盘符加冒号、开头斜线、`./`、`../`、`~/`、`file:`、`%变量%`、`$env:`、`${变量}`；单独含反斜线的文本也一律按路径意图保守处理。识别到路径意图后，只接受未加引号、没有外围空白的 ASCII 盘符绝对路径；UNC、设备路径、根相对、盘符相对、变量表达式等直接报 `invalid_input`。不自动去引号、去空白、补盘符或修正非法路径。普通 HTTP(S) URL 仍是搜索词，不直接导航输入中的 URL。不做自然语言路径提取；要搜索包含 Windows 路径或反斜线正则表达式的句子，使用明确的 `web-search` 动作。

动作只负责分支识别，完整路径词法校验和平台限制由 SDK 与原生接口复验：只允许普通本地固定磁盘，不支持网络映射、SUBST、可移动盘、链接/联接点；云盘名称、云端占位或召回属性会明确拒绝，不能把错误当作 `missing`。访问拒绝、`cancelled`、`timeout`、`fs_busy` 等均保留结构化错误，不会悄悄提交网页搜索。错误和返回摘要不包含路径或搜索词。

调用示例均为合成数据。`open-or-search-call.json` 是明确文本搜索；`open-file-call.json` 使用未检查是否存在的合成路径，仅展示文件输入形式，执行时会先检查元数据，不创建该文件；如果不存在，会按上述规则搜索这个合成路径。可将整个包放入配置同级 `actions/common-search` 并重新加载应用后执行：

```powershell
omnitrigger.exe --run-action <open-or-search-call.json 的绝对路径>
```

通用调用格式：

```json
{
  "package_id": "examples.common-search",
  "action_id": "open-or-search",
  "action_version": 1,
  "input": {"source": "text", "text": "合成示例搜索词"}
}
```

文件分支返回 `{submitted: true, source, operation: "open-file"}`；搜索分支返回 `{submitted: true, source, operation: "search", queryCodePoints, truncated}`。目录/缺失项的搜索额外返回 `pathKind: "directory" | "missing"`，不返回路径本身。`submitted` 仅表示系统接纳打开请求，不能证明文件程序启动、文件内容成功加载或网页完成搜索。

`stat` 是只读元数据快照，与后续打开不是原子事务；本动作不会声称检查后文件无法被替换。文件打开仍受 `ctx.open` 的平台校验约束。若前面已经复制选区，即使随后 `fs.stat` 返回 `effectState: "not_started"`，复制结果仍可能留在剪贴板；该字段只描述失败的那次 API，不代表整个动作没有前序效果。

搜索默认最多保留 128 个 Unicode 码点，可通过 `maxCodePoints` 设置；完整文本先经过 Unicode 校验，截断不会拆开代理对。并发调用受清单中的动作级单实例约束，复制超时和重试仍由宿主 API 决定。动作不读取文件内容、不枚举目录、不监控剪贴板、不创建调度任务。

定向测试只使用内存元数据与合成选区：

```powershell
node --test examples/actions/common-search/recipes.test.mjs examples/actions/common-search/open-or-search.test.mjs
node tools/script-sdk-check/node_modules/typescript/lib/tsc.js -p examples/actions/common-search/jsconfig.json
```
