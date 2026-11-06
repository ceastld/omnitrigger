# 首批脚本 API

状态：可运行的同步原型，不是正式插件。两种引擎加载同一份 `src/sdk.js`，通过 JSON 桥接到 Rust。桌面 API 与 Node 共用参数校验定义，应用接入时经每次调用独立的回环连接交给现有 `NativeHost`；独立 CLI 的剪贴板和打开目标复用 `omnitrigger-windows`。不安装 Hook，不访问 GPUI。`ctx.apiVersion` 为 `1`。[类型声明](../../crates/omnitrigger-actions/runtime/script-api.d.ts)用于编辑器补全，不提供 TS 转译。

Windows MSVC 的 Boa 宿主为共享 SDK 的解析保留 8 MiB 原生主栈；这不是预先提交的全部内存，也不改变 JS 的循环、递归和动作时限。QuickJS 的配置保持原值。

通过 OmniTrigger 的 `script` 动作运行时接入时，`.mjs` 文件具名导出同步 `main(ctx)` 并可导入包内相对模块；已有 `.js` 文件声明 `function main(ctx)`，适配层继续将它转为函数体请求，文件本身保持标准 JavaScript。模块契约见[包内模块](MODULES.md)。`ctx.packageRoot` 为动作包目录，`ctx.invocation` 包含十进制字符串调用身份。独立 CLI 可以省略这些元数据，此时两项为 `null`。成功与失败响应均带有 `schema_version: 1`。

| API | 行为 |
| --- | --- |
| `ctx.fs.readText(path)` | 读取绝对路径的 UTF-8 文本，最多 64 KiB。 |
| `ctx.fs.writeText(path, text, {overwrite})` | 默认创建新文件，目标存在则报错；只有显式 `overwrite:true` 才覆盖，不自动创建目录。 |
| `ctx.process.run(program, args, {timeoutMs})` | 程序和参数数组分别传递，不隐式经过 Shell。返回 `{pid, exitCode, success}`；默认等待 1000 毫秒，允许 1–1000 毫秒。非零退出由脚本判断，stdout / stderr 不采集。 |
| `ctx.process.capture(program, args?, options?)` | 同步采集 Windows 程序输出，返回 `{code, stdout, stderr}`；默认预算 10 秒、默认拒绝非零退出，双流合计最多 64 KiB，严格 UTF-8。详见[程序输出采集](#程序输出采集)。 |
| `ctx.process.start(program, args)` | 创建独立程序后返回 `{pid}`，程序不随脚本结束关闭。 |
| `ctx.delay(milliseconds)` | 同步等待 0–60000 毫秒的整数，返回 `undefined`；字符串、小数、非有限数等返回 `invalid_input`，不做隐式转换。 |
| `ctx.ui.choose({items, title?, placeholder?, timeoutMs?})` | 应用 GPUI 界面中的可搜索选择菜单，返回稳定 ID 或明确取消；Node 入口使用 `await`。独立 CLI 返回 `ui_unavailable`。参数与示例见[脚本选择菜单](../../examples/actions/script-choice/)。 |
| `ctx.desktop.target()` / `targetContext()` | 查询提交时捕获的原目标或导出 29 秒有效的目标快照；为调用方时钟粒度预留余量，提交校验上限仍为 30 秒。不重新选择前台。仅应用接入可用。 |
| `ctx.desktop.readSelectionText(options?)` | 只读原目标中当前焦点控件的单一 UIA 选区，返回 `{text,truncated}`。默认拒绝超过 128 UTF-16 单元的文本，显式 `overflow:'truncate'` 截取前缀。仅应用接入可用。 |
| `ctx.desktop.captureForegroundTarget(options)` | 按明确 PID、FILETIME 字符串和 1–5000 毫秒有效期，捕获该进程当前前台；返回当前动作内的只读目标引用。仅应用接入可用。 |
| `ctx.desktop.sendShortcutTo(target, keys, options?)` / `sendTextTo(target, text)` | 对捕获引用发送快捷键或 Unicode 文本；逐步复验身份、前台、取消和期限，原 `desktop` 目标保持不变。快捷键仅支持可选 `keyDownDelayMs`。 |
| `ctx.desktop.sendShortcut(keys, options)` | 1–5 个 VK 整数，修饰键在前；支持 `keyDownDelayMs` 或原生 `timing` 配置，二者互斥。 |
| `ctx.desktop.sendKeySequence(modifiers, keys, options)` | 0–4 个修饰键和 1–16 个普通键；修饰键跨越整组，`keyDownDelayMs` 为 0–1000 毫秒，单次等待合计最多 3000 毫秒。 |
| `ctx.desktop.sendText(text)` | 向原目标直接发送 Unicode，不经过剪贴板；最多 65,536 个 UTF-16 单元，不含 NUL 或孤立代理项。 |
| `ctx.desktop.copySelectionText()` | 向原目标发送 Ctrl+C，读取新文本；复制结果保留在剪贴板。 |
| `ctx.desktop.copySelectionData(options)` | 向原目标发送一次 Ctrl+C，按可选等待参数返回文本或文件列表与 `changed`；需要应用本机桥接。 |
| `ctx.desktop.windowAction(operation)` | 对仍为前台的原目标执行 `toggle_maximize`、`toggle_topmost`、`minimize` 或 `close`，返回 `{submitted:true}`，不重新选择窗口。 |
| `ctx.clipboard.readText()` / `writeText(text, options)` | Windows Unicode 文本剪贴板；写入替换全部格式。应用接入支持 `{excludeFromHistory:true}`，返回写入回执；独立 CLI 不支持选项，写入返回 `undefined`，文本最多 64 KiB UTF-8。 |
| `ctx.clipboard.readData()` | 应用接入返回 `{kind:'text',text,sequenceNumber}` 或 `{kind:'files',paths,sequenceNumber}`；无桥接明确报错。 |
| `ctx.clipboard.history.list/get/append/remove` | 应用接入的显式纯文本历史；此处同步返回，默认不监听。独立CLI返回 `history_unavailable`。 |
| `ctx.browser.profiles({timeoutMs})` | 查询连接的浏览器 profile。 |
| `ctx.browser.call(method, args, {profile, timeoutMs})` / `ctx.browser.chrome.*` | 同步调用扩展支持的 Chrome API，保留 `undefined` 与 `null`；无 profile 时只接受恰好一个已连接 profile。 |
| `ctx.open(target)` | 系统默认程序打开 http、https、mailto、about 或本地绝对路径；成功只表示系统接纳请求。 |
| `ctx.openFileInParent(target)` | Windows 上复验普通本地文件与父目录，以父目录为工作目录单次打开；同步返回，系统接纳不证明关联程序已完成加载。 |

原生错误转换为 JS 异常，可在 `catch (error)` 中读取 `error.code`，例如 `invalid_arguments`、`file_read_failed`、`file_write_failed`、`process_start_failed` 和 `process_timeout`。通过 SDK 每个脚本最多调用 32 次 API，文件和进程桥接请求最多 256 KiB，桌面桥接最多 512 KiB；桌面与应用接入的剪贴板文本最多 65,536 个 UTF-16 单元，文件文本最多 64 KiB UTF-8。SDK 计数是防误用措施，不是安全隔离。

## 只读文本选区

```js
const selection = ctx.desktop.readSelectionText({overflow: 'truncate'});
return selection.text === ''
  ? {empty: true}
  : {text: selection.text, truncated: selection.truncated};
```

Node 使用相同参数并 `await`。已确认的空选区返回空字符串；provider 不可用、受保护、多选区、目标或选区变化、取消及超时均报错。只检查原提交窗口及焦点祖先链，不激活窗口或读取剪贴板，不模拟复制、不使用 DocumentRange 读取整篇，也不自动重试。截断使用前 128 个 UTF-16 单元，拆开的代理对和 NUL 拒绝。

UIA 在同 exe 的单次辅助进程中执行，复用创建期 Job 与有界采集收尾，读取期限不超过 2 秒并受动作期限约束。无法确认辅助进程收尾时返回 `selection_helper_cleanup_failed`，不交付部分输出；结束本地 helper 不会撤销外部 provider 已开始的工作。`GetText(129)` 仅是请求上限。该 API 共用原调用预算，需要更新应用与相应宿主；独立引擎 CLI 无应用目标桥接时明确失败。

## 显式捕获目标

```js
const target = ctx.desktop.captureForegroundTarget({
  expectedProcessId: ctx.input.processId,
  expectedProcessCreatedFileTime: ctx.input.processCreatedFileTime,
  validForMs: 500,
});
ctx.desktop.sendTextTo(target, '示例文本');
return { submitted: true };
```

调用方必须明确提供预期进程身份；Node 使用相同参数并逐次 `await`。每次有效参数捕获尝试都会替换上一条新目标引用，失败也不回退旧引用。新引用不改变原 `targetContext()`、`sendShortcut()` 或 `sendText()` 的目标，不能跨动作保存或通过改写期限续期。新方法共用已有调用预算，没有另授额度。

捕获不枚举、不激活窗口，不把进程身份当作浏览器 `windowId` 到 HWND 的关联。发送返回 `{submitted:true}` 仅确认系统接纳输入；输入前明确拒绝为 `not_started`，进入平台提交后未获确认的失败为 `unknown`。租约使用宿主单调时钟并受动作总期限约束，不自动重试。应用和轻量宿主须同时具备这些入口；独立引擎 CLI 没有应用目标桥接时明确报错。

## 程序输出采集

`ctx.process.capture` 在 Windows 上共用一份 Rust 采集后端：Boa、QuickJS 同步返回，Node 动作通过应用原生桥接返回 Promise，须逐次 `await`。需要更新应用和相应宿主；类型声明或示例更新不能为旧构建增加能力。Node 无原生桥接时明确失败，不回退 `run`。已有两种程序输出配方见[程序输出通知动作](../../examples/actions/process-output/)。

```js
const result = ctx.process.capture('C:\\OwnedFixture\\program.exe', ['--format', 'json'], {
  cwd: 'C:\\OwnedFixture',
  timeoutMs: 10000,
  allowNonZero: false,
});
return { code: result.code, stdoutLength: result.stdout.length };
```

上述路径为占位示例，需要替换为明确的自有程序和目录。Node 使用同一参数，将调用改为 `await ctx.process.capture(...)`；返回字段相同。

| 参数或结果 | 契约 |
| --- | --- |
| `program` | 本地盘符绝对 `.exe` 路径，不查找 `PATH`，不隐式调用 Shell；拒绝 UNC、设备路径和 `.` / `..` 路径段。脚本须显式指定解释器 EXE 与参数，不能直接传入 `.cmd` / `.bat`。 |
| `args` | 默认 `[]`，最多 128 个字符串；保留参数边界，按 Windows 命令行规则编码。程序和参数须为有效 Unicode，不含 NUL；完整命令行连同结束 NUL 最多 32,767 个 UTF-16 单元。 |
| `options.cwd` | 可选的本地盘符绝对目录路径，拒绝 `.` / `..`、NUL 和非字符串；Node 省略时使用动作包目录；轻量宿主省略时继承宿主工作目录。需要稳定目录时显式设置。 |
| `options.timeoutMs` | 1–60,000 的整数，默认 10,000 毫秒。未知选项、错误类型或非法值在启动前拒绝，不隐式转换。 |
| `options.allowNonZero` | 布尔值，默认 `false`；非零退出抛出 `process_exit_nonzero`。设为 `true` 才返回该退出码与完整输出。 |
| 返回值 | 恰含 `{code, stdout, stderr}`，`code` 为有符号 32 位退出码。两个流的原始字节合计最多 65,536，包含 stderr；等待直接进程退出和双流 EOF 后分别严格解码 UTF-8，不补替换字符。NUL 是合法输出字符，后续显示接口可有更严格限制。 |

超出字节上限返回 `process_output_too_large`，无效 UTF-8 返回 `process_invalid_utf8`；错误不返回部分输出。双流交替采集，后代持有管道导致 EOF 未到也受预算约束。`timeoutMs` 是在创建、采集和等待阶段观察的预算，不能中断阻塞中的操作系统调用，不是硬实时超时；收尾可能额外等待约一秒。通过动作服务调用时，外层动作总时限可能更短，并会终止直接脚本宿主；独立 CLI 没有这层动作期限。

采集使用 Windows 10+ 私有 Job，在进程创建时关联，收尾时终止 Job 内进程并检查直接进程退出及 Job 内活动进程归零，再交付结果。轻量宿主持有自己的 Job，宿主被强制终止后由操作系统关闭句柄并终止其中进程。Node 的 Job 由应用持有；Node 退出或断连由桥接检测并请求协作取消和收尾，不能等同于轻量宿主的操作系统直接关闭保证。Job 建立或关联失败即报错，不回退到无 Job 启动；不支持的平台返回 `unsupported_platform`。该范围仅覆盖 Job 内进程，不能保证外部服务或未包含的进程结束，也不能撤销程序已经产生的副作用；现有 `start` 独立程序的生命周期保持不变。

SDK 异常固定 `method: 'process.capture'`，消息为“程序输出采集未完成，请检查错误代码。”，不回显路径、参数或输出。JS 使用 `effectState`，原生协议使用 `effect_state`：确认尚未启动为 `not_started`，已启动后失败或效果不确定为 `unknown`。常见错误如下；脚本应按代码处理，不自动重试。

| 错误代码 | 含义与效果状态 |
| --- | --- |
| `invalid_input` / `invalid_arguments` / `request_too_large` | SDK 参数、原生参数或请求超限，`not_started`。 |
| `unsupported_platform` / `process_setup_failed` / `process_start_failed` | 平台不支持、Job/管道设置或进程创建失败，`not_started`。 |
| `cancelled` / `timeout` | 外层动作取消或期限到达；启动前为 `not_started`，启动后为 `unknown`。 |
| `process_timeout` / `process_output_too_large` / `process_invalid_utf8` / `process_exit_nonzero` | 启动后超时、超限、解码或非零策略失败，`unknown`。 |
| `process_read_failed` / `process_wait_failed` / `process_cleanup_failed` | 采集、等待或收尾失败，`unknown`，不能据此断言副作用未发生。 |
| `process_clock_error` | 启动前预算计算失败为 `not_started`，采集中计时异常为 `unknown`。 |
| `invalid_response` / `process_capture_failed` | 回执无效或未识别的采集失败；派发后的效果不确定时为 `unknown`。 |

Node 的 `capture` 须等待完成；未 `await` 的在途采集会令动作报 `pending_process_capture`，并关闭连接以请求收尾。所有本机调用共用在途限制与每动作128次计数，采集的传输预算为 `timeoutMs + 1500ms`、最多61500ms，仍受更短的外层期限约束。

与 Node 现有 `await ctx.process.run(program, args, {cwd, allowNonZero})` 相比，返回字段相同，但契约不同：Node 旧 `run` 没有单次 `timeoutMs`，省略 `cwd` 使用包目录，输出按 UTF-8 替换无效字节；取消处理直接子进程，没有上述私有 Job 收尾保证。Node 旧 `run` 的双流同样合计最多 64 KiB。这些区别不能通过改名调用消除；轻量旧 `run` 仍返回 `{pid, exitCode, success}`，不采集输出。

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

桌面异常的 `code`、`method`、`effectState` 可在 JS 中捕获，未捕获时保留在动作终态。`not_started` 仅描述该次 API，不能证明此前脚本没有副作用；网络中断或无法确认是否执行时为 `unknown`。桥接失败不自动重试，也不退回宿主直接访问剪贴板。客户端每次连接只发送一帧，普通桌面请求最多等待 5 秒，`copySelectionData` 为 8 秒；交互选择和浏览器请求另有等待期限，外层动作仍受总时限约束。

## 选区数据

`copySelectionData({timeoutMs:400,pollIntervalMs:10,settleMs:0,allowUnchanged:false})` 可显式设置等待序号变化、轮询和读前等待。默认分别为 600/15/30ms，未变化时报 `selection_unchanged`；只有显式 `allowUnchanged:true` 才可返回未变化的数据。复制只发一次，结果留在剪贴板，不抢焦点、不重选目标、不恢复原内容。 format 可显式设为 auto/text/files，默认 auto 保持文本优先（含空文本）；text/files 仅读取指定格式，缺失或无效时明确失败，不回退另一格式。format 在发送 Ctrl+C 前校验；readData() 保持无参且文本优先。等待预算不保证阻塞 Win32 调用的硬期限，`changed` 只表示序号不同，不证明数据由目标生成。

读取文本优先，允许空文本；文件列表支持 `[]`，每项必须非空。文本和路径总长各最多 65,536 个 UTF-16 单元，路径最多 64 项，不含 NUL 或孤立代理项。路径仅作为数据，不检查或打开文件；无支持格式、畸形格式或超限均明确报错。完整参数和示例见[动作包文档](../../docs/ACTION_PACKAGES.md#文本与文件列表)。

## 显式文本历史

应用接入提供以下同步接口；Node入口使用相同对象参数并逐次 `await`：

| 方法 | 结果 |
| --- | --- |
| `ctx.clipboard.history.list({limit?})` | 默认100项、范围1–200；返回 `{items:[{id,preview,utf16Units,createdAt}],revision}`，不含全文。也可省略整个参数。 |
| `ctx.clipboard.history.get({id})` | 返回 `{id,text,createdAt,revision}`；不存在抛 `history_item_missing`。 |
| `ctx.clipboard.history.append({text})` | 显式保存文本，返回 `{id,revision,inserted}`；空文本和空白保留，与最新条目完全相同可去重。 |
| `ctx.clipboard.history.remove({id})` | 返回 `{removed,revision}`；不存在为false，未变不增加revision。 |

ID必须是非零、无前导零的十进制u64字符串；revision也是十进制字符串，可为 `"0"`，不转JS数字。createdAt为Unix毫秒字符串。单条原文最多65536个UTF-16单元，有效Unicode、无NUL；最多200项、总长1048576个UTF-16单元，超出时按最旧项逐条淘汰，不截断原文。列表摘要单行、最多64个UTF-16单元且不切断代理对；utf16Units始终表示原文长度。

存储是应用配置同级 `clipboard-history.json` 中的明文JSON，读取上限8MiB；损坏或未知版本明确拒绝并保留原文件。默认不启动监听，不读取当前剪贴板或导入其他历史；构造宿主不读写文件，只有显式调用才访问存储。历史方法不需要目标窗口，独立CLI无应用桥接时四方法仍存在并抛 `history_unavailable`。

错误提供固定正文与 `code/method/effectState`，不附带历史正文、路径或权限字段。参数/结果错误为 `invalid_input/invalid_response`，不存在为 `history_item_missing`；锁竞争、损坏和外部变化分别为 `history_busy/history_corrupt/history_conflict`，计数溢出为 `history_id_exhausted/history_revision_exhausted`，取消/期限为 `cancelled/timeout`。读失败为not_started，写入提交前明确拒绝为not_started，提交后无法确认时为unknown；不自动重试或回滚。5秒客户端传输期限不保证抢占已进入的文件系统调用，外层动作取消和期限仍优先生效。

`get`正文仅供脚本内部处理；应返回状态、数量和长度等摘要。SDK不会自动删改可信脚本主动返回的全文。参数、完整边界及双入口示例见[动作包文档](../../docs/ACTION_PACKAGES.md#显式文本历史)和[显式历史配方](../../examples/actions/clipboard-history/)。已安装旧宿主与已有工程的SDK快照需要更新后才能使用。

## 窗口动作

`toggle_topmost` 按本次原窗口提交前的置顶状态切换；不激活、不移动或缩放窗口。此调用沿用动作总期限和取消条件，提交后迟到或取消不撤销已提交操作。

```js
/** @param {ScriptContext} ctx */
function main(ctx) {
    return ctx.desktop.windowAction('toggle_maximize');
}
```

此接口复用原生窗口适配，不模拟按键；Node 入口使用 `await ctx.desktop.windowAction(...)`。状态检查与提交前重复核验原窗口、线程及进程创建身份、前台和取消条件；这些检查不构成对 Windows 窗口生命周期的原子锁定。非法操作在调用前返回 `invalid_input`。原目标已变时停止，系统请求未确认时返回 `window_action_failed` 并标记 `unknown`，不会自动重试。

`submitted` 只表示系统接纳异步请求，不保证界面已经变化；关闭窗口可能弹出保存提示或被应用拒绝。取消不能撤销已提交请求。独立 CLI 没有桌面上下文时返回 `desktop_unavailable`；使用新增 API 需要更新宿主开发包。

## 顺序等待

```js
/** @param {ScriptContext} ctx */
function main(ctx) {
    ctx.delay(80);
    return {ready: true};
}
```

等待在独立宿主中休眠，计入每次脚本最多 32 次 API 调用，不使用 CPU 忙等。它只保证经过等待时间，不表示外部窗口或网页已就绪。通过 OmniTrigger 运行时，等待计入动作总时限；取消和超时会终止并回收直接宿主，后续脚本步骤不再继续，此前已发生的副作用保留。独立 CLI 没有外层动作服务的取消机制。新增 API 需要使用包含此能力的宿主开发包；修改 SDK 声明不会升级已安装宿主。

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
