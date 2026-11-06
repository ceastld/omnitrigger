# 创建和刷新 JavaScript 工程

应用界面的“新建脚本”创建同步轻量工程。开发命令支持创建 Node 或轻量工程，并显式刷新已有工程的 SDK 声明。它复用应用的工程创建器，不运行作者代码。

## 创建工程

在仓库根目录使用已经构建的应用。以下 PowerShell 命令创建独立配置和两个新工程：

```powershell
$app = (Resolve-Path 'target/debug/omnitrigger.exe').Path
$root = Join-Path (Get-Location).Path '.tmp/script-start'
$config = Join-Path $root 'bindings.json'
if (Test-Path -LiteralPath $root) { throw '请选择新的演示目录。' }
New-Item -ItemType Directory -Path $root | Out-Null
[IO.File]::WriteAllText($config, '{"version":23,"bindings":[]}', [Text.UTF8Encoding]::new($false))

node tools/script-project.mjs create --runtime node --binary $app --config $config --id node-demo
node tools/script-project.mjs create --runtime script --binary $app --config $config --id script-demo
```

工程位于配置旁的 `actions/<标识>/`。入口、包内模块、调用文件、SDK 和 `jsconfig.json` 一并生成；已有同名工程不会被覆盖。Node 工程另有声明 ESM 格式的 `package.json`。

应用与仓库 SDK 应来自同一构建版本。创建时会比对应用实际生成的同步声明；不一致返回 `project_sdk_binary_mismatch`，保留刚生成的轻量工程，不继续改成 Node。对齐版本后选择新标识创建。

## 入口和相对模块

| 运行时 | ESM 入口 | SDK 调用 |
| --- | --- | --- |
| Node | `export default async function main(ctx)` | 返回 Promise，需要等待结果 |
| 轻量脚本 | `export function main(ctx)`，使用 `ScriptMain` 类型 | 同步返回 |

Node 的入口必须默认导出函数。只命名导出 `main` 即使能通过类型检查，正式 NodeRuntime 仍会拒绝为 `invalid_entry`。

Node 工程的默认入口只整理输入文本，使用包内相对模块：

```javascript
import { cleanText } from './lib/text.js';

/** @param {import('./sdk/api').ActionContext} ctx */
export default async function main(ctx) {
  return { text: cleanText(ctx.input.text), invocation: ctx.invocation };
}
```

`lib/text.js` 可以继续命名导出辅助函数；默认导出的要求针对动作入口。相对导入须保留包内路径和文件扩展名。

## 通过正式 NodeRuntime 运行

`--check-action` 检查调用引用，不导入入口；入口导出形式需要实际运行核对。对于上述只整理文本的默认工程，可运行生成的 `call.json`。

Windows GUI 子系统程序需要等待退出并读取两条输出流。下面通过 Node 等待已构建应用，执行正式 `--run-action` 路径：

```powershell
$run = @'
import {spawnSync} from "node:child_process";
const r = spawnSync(process.argv[1], process.argv.slice(2), {
  encoding:"utf8", windowsHide:true, timeout:30000
});
if (r.error) throw r.error;
process.stdout.write(r.stdout || "");
process.stderr.write(r.stderr || "");
process.exit(r.status ?? 1);
'@
$call = Join-Path $root 'actions/node-demo/call.json'
node --input-type=module -e $run -- $app --run-action $call --config $config
```

成功回执的 `status` 为 `succeeded`，`output.text` 为整理后的文本。调用文件的 `input` 可改为自己的合成文本。

## 捕获目标 API 的类型和等待

Node 的三项调用都需要等待结果：

```javascript
const target = await ctx.desktop.captureForegroundTarget({
  expectedProcessId: Number(ctx.input.expectedProcessId),
  expectedProcessCreatedFileTime: String(ctx.input.expectedProcessCreatedFileTime),
  validForMs: 1000,
});
await ctx.desktop.sendShortcutTo(target, [0x11, 0x41]);
await ctx.desktop.sendTextTo(target, String(ctx.input.text));
```

这段代码发送真实输入，目标身份须来自调用者明确选定的窗口。捕获结果及其内部上下文是只读的；不要修改、伪造或保存后复用。句柄和进程创建时间保持字符串精度。两项发送返回的 `submitted` 只表示系统接纳请求。

轻量工程使用相同调用顺序，移除 `await`，入口保持普通同步函数。类型检查会拒绝把 Node 中未等待的 Promise 当作捕获目标或发送回执；它无法阻止作者主动忽略 Promise。

## 刷新已有工程

```powershell
node tools/script-project.mjs refresh --runtime node --project "$root/actions/node-demo"
node tools/script-project.mjs refresh --runtime script --project "$root/actions/script-demo"
```

刷新只替换相应 SDK 声明，保留入口、包内模块、调用文件、清单、作者配置和说明。Node 同时分发 `api.d.ts` 与其引用的 `clipboard-history-api.d.ts`。指定运行时必须与工程中的动作一致；混合运行时工程会被拒绝，避免声明冲突。

各声明文件分别替换，刷新不是跨文件事务。中途失败可能留下部分已更新的 SDK；作者文件保持不变，排除原因后可显式再次刷新。

使用与实际应用版本配套的声明来源。默认来源是当前仓库；`--source-root` 是显式开发接口。界面的“重新加载”只刷新动作目录，不等同于此命令的 SDK 刷新。

## 本地离线类型检查

校验使用仓库现有的 TypeScript 依赖。安装一次本地开发依赖后，类型检查不需要网络或全局 `node_modules`：

```powershell
npm ci --prefix tools/script-sdk-check --ignore-scripts
node tools/script-sdk-check/node_modules/typescript/lib/tsc.js --project "$root/actions/node-demo/jsconfig.json" --noEmit --skipLibCheck false --pretty false
node tools/script-sdk-check/node_modules/typescript/lib/tsc.js --project "$root/actions/script-demo/jsconfig.json" --noEmit --skipLibCheck false --pretty false
```

生成的 `jsconfig.json` 开启 `checkJs`、严格检查和无输出模式。SDK 的只读捕获类型、Node Promise 返回值和相对模块导入均参与检查；运行时入口核对仍使用前面的正式调用命令。


## 取消信号与环境类型

Node 的 `ctx.signal` 使用 SDK 自有的 `ActionCancellationSignal` 观察接口：只读 `aborted`、`reason`，以及 `throwIfAborted()`、`onabort`、`abort` 事件的添加和移除。事件只描述取消事件的类型和信号目标，不要求 DOM 的 `EventTarget` 类型。

```javascript
ctx.signal.throwIfAborted();
const onAbort = () => { /* 停止本次调用的后续工作 */ };
ctx.signal.addEventListener('abort', onAbort, {once: true});
// 自定义异步步骤结束时移除监听，避免保留不必要的引用。
ctx.signal.removeEventListener('abort', onAbort);
```

运行时仍传递原生取消信号；声明只覆盖 SDK 的取消观察契约。SDK 不引用完整 DOM 库，因此不会为 Node 工程添加 `window`、`document` 等浏览器全局，也不会给同步轻量工程添加 `process`、`Buffer`、`require` 等 Node 全局。作者若另外需要完整 Node 开发类型，应在自己的 Node 工程中显式配置；SDK 刷新不修改作者的 `jsconfig.json`。

工程已经提供原生 `AbortSignal` 声明时，`ctx.signal` 沿用该类型，继续兼容需要原生信号的函数；没有环境声明时才使用独立的观察接口。SDK 不会为了取得信号类型自动加入环境库。
