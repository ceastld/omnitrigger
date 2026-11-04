# 首批脚本 API

状态：可运行的同步原型，不是正式插件。两种引擎加载同一份 `src/sdk.js`，通过 JSON 桥接到 Rust。桌面 API 与 Node 共用参数校验定义，应用接入时经每次调用独立的回环连接交给现有 `NativeHost`；独立 CLI 的剪贴板和打开目标复用 `omnitrigger-windows`。不安装 Hook，不访问 GPUI。`ctx.apiVersion` 为 `1`。[类型声明](../../crates/omnitrigger-actions/runtime/script-api.d.ts)用于编辑器补全，不提供 TS 转译。

通过 OmniTrigger 的 `script` 动作运行时接入时，文件声明 `function main(ctx)`；适配层将它转为本工具的函数体请求，文件本身保持标准 JavaScript。`ctx.packageRoot` 为动作包目录，`ctx.invocation` 包含十进制字符串调用身份。独立 CLI 可以省略这些元数据，此时两项为 `null`。成功与失败响应均带有 `schema_version: 1`。

| API | 行为 |
| --- | --- |
| `ctx.fs.readText(path)` | 读取绝对路径的 UTF-8 文本，最多 64 KiB。 |
| `ctx.fs.writeText(path, text, {overwrite})` | 默认创建新文件，目标存在则报错；只有显式 `overwrite:true` 才覆盖，不自动创建目录。 |
| `ctx.process.run(program, args, {timeoutMs})` | 程序和参数数组分别传递，不隐式经过 Shell。返回 `{pid, exitCode, success}`；默认等待 1000 毫秒，允许 1–1000 毫秒。非零退出由脚本判断，stdout / stderr 不采集。 |
| `ctx.process.start(program, args)` | 创建独立程序后返回 `{pid}`，程序不随脚本结束关闭。 |
| `ctx.desktop.target()` / `targetContext()` | 查询提交时捕获的原目标或导出 30 秒有效的目标快照；不重新选择前台。仅应用接入可用。 |
| `ctx.desktop.sendShortcut(keys, options)` | 1–5 个 VK 整数，修饰键在前；支持 `keyDownDelayMs` 或原生 `timing` 配置，二者互斥。 |
| `ctx.desktop.sendKeySequence(modifiers, keys, options)` | 0–4 个修饰键和 1–16 个普通键；修饰键跨越整组，`keyDownDelayMs` 为 0–1000 毫秒，单次等待合计最多 3000 毫秒。 |
| `ctx.desktop.sendText(text)` | 向原目标直接发送 Unicode，不经过剪贴板；最多 65,536 个 UTF-16 单元，不含 NUL 或孤立代理项。 |
| `ctx.desktop.copySelectionText()` | 向原目标发送 Ctrl+C，读取新文本；复制结果保留在剪贴板。 |
| `ctx.clipboard.readText()` / `writeText(text, options)` | Windows Unicode 文本剪贴板；写入替换全部格式。应用接入支持 `{excludeFromHistory:true}`，返回写入回执；独立 CLI 不支持选项，写入返回 `undefined`，文本最多 64 KiB UTF-8。 |
| `ctx.browser.profiles({timeoutMs})` | 查询连接的浏览器 profile。 |
| `ctx.browser.call(method, args, {profile, timeoutMs})` / `ctx.browser.chrome.*` | 同步调用扩展支持的 Chrome API，保留 `undefined` 与 `null`；无 profile 时只接受恰好一个已连接 profile。 |
| `ctx.open(target)` | 系统默认程序打开 http、https、mailto、about 或本地绝对路径；成功只表示系统接纳请求。 |

原生错误转换为 JS 异常，可在 `catch (error)` 中读取 `error.code`，例如 `invalid_arguments`、`file_read_failed`、`file_write_failed`、`process_start_failed` 和 `process_timeout`。通过 SDK 每个脚本最多调用 32 次 API，文件和进程桥接请求最多 256 KiB，桌面桥接最多 512 KiB；桌面与应用接入的剪贴板文本最多 65,536 个 UTF-16 单元，文件文本最多 64 KiB UTF-8。SDK 计数是防误用措施，不是安全隔离。

## 原目标快捷键

动作包中的同步 JS 入口可以直接组合 API，无需加载平台模块：

```js
/** @param {ScriptContext} ctx */
function main(ctx) {
    const target = ctx.desktop.target();
    const result = ctx.desktop.sendShortcut([162, 67], {keyDownDelayMs: 20});
    return {target, submitted: result.submitted};
}
```

`target()` 只查询原目标；发送前由应用再次核验进程身份、前台和取消状态，并复用现有按键释放收尾逻辑。`submitted` 只表示系统接纳输入，不表示业务动作完成。独立 CLI 没有目标上下文时返回 `desktop_unavailable`，不会自行选择前台。

桌面异常的 `code`、`method`、`effectState` 可在 JS 中捕获，未捕获时保留在动作终态。`not_started` 仅描述该次 API，不能证明此前脚本没有副作用；网络中断或无法确认是否执行时为 `unknown`。桥接失败不自动重试，也不退回宿主直接访问剪贴板。客户端每次连接只发送一帧，最多等待 5 秒；外层动作仍受总时限约束。

## 文件清理动作

[完整入口](examples/clean-file.js)读取文本、清除行尾空白、生成新文件，使用普通 JS 完成处理：

```js
const text = ctx.fs.readText(ctx.input.source);
const cleaned = text.replace(/\r\n?/g, '\n').replace(/[\t ]+$/gm, '');
ctx.fs.writeText(ctx.input.destination, cleaned);
return { destination: ctx.input.destination, characters: cleaned.length };
```

从本目录执行（需提供真实输入文件和不存在的输出路径）：

```powershell
@{
    script = Get-Content ./examples/clean-file.js -Raw
    input = @{ source = 'D:/Temp/input.txt'; destination = 'D:/Temp/cleaned.txt' }
} | ConvertTo-Json -Depth 8 -Compress | ./results/boa-small.exe
```

## 剪贴板清理动作

[完整入口](examples/clean-clipboard.js)读取文本，清理行尾空白，写回剪贴板：

```js
const text = ctx.clipboard.readText();
const cleaned = text.replace(/\r\n?/g, '\n').replace(/[\t ]+$/gm, '');
ctx.clipboard.writeText(cleaned);
return { before: text.length, after: cleaned.length };
```

将上述入口作为请求的 `script`、`input` 设置为 `{}` 后执行，会真实修改剪贴板。

## 当前执行边界

- API 同步运行在独立宿主进程中，不进入输入 Hook。原生能力由 Rust 提供，JS SDK 和引擎适配分别演进。
- API 使用当前用户权限，文件路径与 JSON 桥接不是沙盒。文件覆盖、剪贴板修改和外部程序副作用不会回滚，写入失败可能留下部分内容。
- `run` 的等待期限从进程创建返回后计算。超时会请求终止并等待直接子进程，不保证它自行创建的后代全部结束。`start` 的独立程序始终保留。
- JS 执行限制不能中断正在阻塞的原生文件或外壳调用，目前没有覆盖所有原生调用的统一墙钟取消。正式接入需要外层执行器的超时与取消管理。
- 目标查询、快捷键、连续按键、文本和选区 API 只通过应用捕获的目标上下文执行。浏览器调用同样经应用宿主复用已安装连接器，独立 CLI 不连接浏览器；尚无鼠标操作或任意窗口控制。

自动测试对临时文件执行真实 JS → Rust → 文件读写，验证默认禁止覆盖、显式覆盖、结构化错误、真实程序退出码、独立启动和超时。剪贴板读取另在本机只读验证；测试不写用户剪贴板，也不打开外部窗口。底层剪贴板和打开目标的替身测试沿用主 workspace 的原生适配测试。

桌面接入测试通过真实 Boa / QuickJS 进程和回环 TCP 连接执行，`NativeHost` 使用记录替身，验证所有方法、Unicode 上限、调用顺序、结构化错误和取消；不向用户窗口注入输入，也不写用户剪贴板。

## 浏览器调用

```js
/** @param {ScriptContext} ctx */
function main(ctx) {
    const profiles = ctx.browser.profiles();
    const tree = ctx.browser.call('bookmarks.getTree', [], {profile: ctx.input.profile});
    return {profiles: profiles.length, roots: tree.length};
}
```

需要在应用中安装、启用并连接浏览器连接器；脚本宿主自身不加载 Node SDK 或浏览器库。Node 和轻量运行时共用浏览器参数校验，但 Node 返回 Promise，轻量接口同步顺序执行。`chrome.*` 适用于默认 profile；显式 profile 或超时使用 `call`。方法不带 `chrome.` 前缀，参数必须为 JSON 数组；拒绝函数、undefined、非有限数字、循环引用及无效 Unicode。

单次 RPC 默认等待 10 秒，允许 1–60000 毫秒；新连接另有最多 5 秒握手等待，均受外层动作时限约束。取消、超时或调用错误后关闭受影响的 Rust 连接，不重试已派发操作。异常保留 `code`、`method`、`effectState`；服务端提供权限详情时保留 `requiredPermissions`。`unknown` 不能理解为操作未发生。

浏览器的原始 JSON 结果上限为 32 MiB，经现有分块协议接收，最终动作结果仍最多 64 KiB。JS 引擎的堆内存和执行限制独立生效，因此传输上限不是可成功处理所有同等大小对象的保证；大结果应在脚本中处理后返回摘要。桌面回执仍限 512 KiB；浏览器回执预留协议封装开销，等待同时受动作总时限限制。
