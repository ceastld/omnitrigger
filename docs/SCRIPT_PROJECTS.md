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

工程位于配置旁的 `actions/<标识>/`。入口、包内模块、调用文件、SDK 和 `jsconfig.json` 一并生成；已有同名工程不会被覆盖。Node 工程另有声明 ESM 格式、Node >=22 基线和固定开发依赖的 `package.json`。创建不安装依赖、不执行 npm 或作者脚本。

应用与仓库 SDK 应来自同一构建版本。创建时会比对应用实际生成的同步声明；不一致返回 `project_sdk_binary_mismatch`，保留刚生成的轻量工程，不继续改成 Node。对齐版本后选择新标识创建。

## 入口和相对模块

| 运行时 | ESM 入口 | SDK 调用 |
| --- | --- | --- |
| Node | `export default async function main(ctx)` | 返回 Promise，需要等待结果 |
| 轻量脚本 | `export function main(ctx)`，使用 `ScriptMain` 类型 | 同步返回 |

Node 的入口必须默认导出可调用函数。新建 Node 工程的 `npm run check` 会先运行严格类型检查，再按 `action.json` 检查实际入口的默认导出与明确不支持的顶层返回类型。检查不执行作者代码，也不输出编译文件。

入口检查会等待类型层面的顶层 Promise：明确的函数、类构造器、symbol 或 bigint 返回类型会报错。空返回、Promise 和普通对象沿用现有宿主行为。动态类型、混合类型、自定义序列化或 CommonJS 入口会报告未确认；静态检查不保证对象内部都可序列化，实际结果仍由正式运行器处理。

旧生成布局的 Node 工程刷新 SDK 后会获得同一检查器，原作者的 `package.json` 不会被改写；可显式运行 `node sdk/check-node-entry.cjs .`。

Node 工程的默认入口只整理输入文本，使用包内相对模块：

```javascript
import { cleanText } from './lib/text.js';

/** @param {import('./sdk/api').ActionContext} ctx */
export default async function main(ctx) {
  return { text: cleanText(ctx.input.text), invocation: ctx.invocation };
}
```

`lib/text.js` 可以继续命名导出辅助函数；默认导出的要求针对动作入口。相对导入须保留包内路径和文件扩展名。

## 编辑器与静态检查入口

在 VS Code 中打开包含 `action.json` 和 `jsconfig.json` 的动作包根文件夹，再打开入口。启用内置 JavaScript/TypeScript 语言功能时，语言服务会自动发现入口所在目录的 `jsconfig.json`；不必把 SDK 声明逐个手动打开。新建 Node 工程先按下文安装本工程开发依赖，轻量工程无需 Node 环境类型。补全依赖入口的 JSDoc 标注：Node 使用 `import('./sdk/api').ActionContext`，轻量模块使用 `ScriptMain`。跳转定义应落到当前包的 SDK，Node API 的补全签名包含 Promise，轻量 API 为同步签名。

每个动作包保留自己的配置和声明快照。不要为了同时编辑多个包而把它们的 SDK 声明汇总到一个 `jsconfig.json`；这会混合不同全局声明或带独立标记的捕获类型。修改代码后的错误应定位到当前入口的行列，例如 Node 未等待 `ctx.desktop.target()` 就读取 `processName`，或调用不存在的 API。

开发工具 `tools/script-project.mjs` 只有 `create` 和 `refresh`，没有 `check` 子命令。Node 工程可继续使用下文的 `npm run check`；Node、轻量或混合普通包及生成工程也可在仓库根使用同一受限静态检查入口：

```powershell
node tools/script-sdk-check/check-project-types.mjs "$root/actions/script-demo"
```

该命令读取指定包自己的 SDK 与配置，输出类型诊断及模块/入口检查结果，不执行作者初始化器或入口。类型诊断的 `file`、`line`、`column` 使用包内路径和从 1 开始的位置。它会按清单运行时分别核验轻量具名导出和 Node 默认导出，Node 结果见 `nodeEntryChecks`。纯 Node 的 `types:["node"]` 只解析包内显式依赖；缺少 Node 环境声明时真实报错。同一配置混合两种运行时时，非空 `types` 仍明确拒绝；需要 Node 内置模块的包可使用[分目录就近配置示例](../examples/actions/mixed-projects/README.md)，让同步入口保持独立环境。完整边界见[工程检查说明](../tools/script-sdk-check/PROJECT_CHECK.md)。静态通过不代表宿主或目标软件已经验收。

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

刷新保留入口、包内模块、调用文件、清单、作者配置和说明。旧生成布局沿用对应运行时的刷新方式：Node 更新 `sdk/api.d.ts`、`sdk/clipboard-history-api.d.ts` 和入口检查器；轻量工程更新 `sdk/api.d.ts`。此分支仍会替换现有声明，指定运行时必须与所有动作一致。

普通示例包若携带完整的 `api.d.ts`、`script-types.d.ts`、`clipboard-history-api.d.ts`，支持这组三文件位于包根或 `sdk/` 的两种布局；`sdk/` 中已存在的 `script-api.d.ts` 兼容声明也一并更新。此分支可以包含 Node 与轻量动作，`--runtime` 必须是包中存在的运行时，刷新会更新同包整组声明，不改成另一种目录布局，也不额外分发入口检查器。

普通包的每份声明须与所选来源相同，或属于工具已识别的随包规范快照。作者改过声明或快照未知时返回 `project_sdk_unrecognized`；缺件返回 `project_sdk_layout_incomplete`；根目录声明与旧 `sdk/api.d.ts` 并存等布局歧义返回 `project_sdk_layout_ambiguous`。这些预检失败均不写入声明。应先核对并保留作者改动，再选择与实际运行版本配套的声明来源。这项归属检查只适用于上述已识别普通包布局，旧生成布局的替换策略不变。

各声明文件分别替换，刷新不是跨文件事务。中途失败可能留下部分已更新的 SDK；作者文件保持不变，排除原因后可显式再次刷新。

使用与实际应用版本配套的声明来源。默认来源是当前仓库；`--source-root` 是显式开发接口。界面的“重新加载”只刷新动作目录，不等同于此命令的 SDK 刷新。

## 本地离线类型检查

新建 Node 工程固定 TypeScript 5.9.3 与 DefinitelyTyped 的 `@types/node 22.20.5` 开发依赖。`jsconfig.json` 保持 ESNext/Bundler 模块检查，使用 ES2022 标准库和 `types: ["node"]`，可严格检查 `node:fs`、`process`、`fetch` 等 Node 22 API，同时拒绝 `window`、`document`。这仅配置 Node 工程，不修改 SDK 的环境类型。

作者首次在工程目录显式安装并检查：

```powershell
Push-Location "$root/actions/node-demo"
npm install --ignore-scripts
npm run check
Pop-Location
```

安装只提供开发检查所需依赖，`--ignore-scripts` 禁止依赖的生命周期脚本。首次安装生成工程自己的 `package-lock.json`；之后可在该工程目录用 `npm ci --ignore-scripts` 复现版本。检查不执行动作代码，安装后无需网络或全局 TypeScript。动作实际运行仍需 Node。

轻量工程只使用仓库现有 TypeScript，不安装或加载 Node 声明：

```powershell
npm ci --prefix tools/script-sdk-check --ignore-scripts
node tools/script-sdk-check/node_modules/typescript/lib/tsc.js --project "$root/actions/script-demo/jsconfig.json" --noEmit --skipLibCheck false --pretty false
```

只使用 SDK API、没有 `npm run check` 脚本的普通 Node 包，可在仓库根显式检查自己的配置和默认导出：

```powershell
$package = "$root/actions/node-demo"
node tools/script-sdk-check/node_modules/typescript/lib/tsc.js --project "$package/jsconfig.json" --noEmit --skipLibCheck false --pretty false
node tools/script-sdk-check/check-node-entry.cjs $package
```

这不会为包安装或补齐环境声明。若作者代码使用 Node 内置 API，仍需按本包配置安装相应开发依赖；SDK 声明本身不提供 `node:fs` 等内置模块或 `process` 等全局类型。

两类工程均开启 `checkJs`、严格检查和无输出模式。SDK 的只读捕获类型、Node Promise 返回值和相对模块导入均参与检查；运行时入口核对仍使用前面的正式调用命令。既有 Node 工程的作者配置不自动迁移，作者可自行添加相同开发依赖与 Node 类型配置；SDK 刷新仍只更新声明。

## 取消信号与环境类型

Node 的 `ctx.signal` 使用 SDK 自有的 `ActionCancellationSignal` 观察接口：只读 `aborted`、`reason`，以及 `throwIfAborted()`、`onabort`、`abort` 事件的添加和移除。事件只描述取消事件的类型和信号目标，不要求 DOM 的 `EventTarget` 类型。

```javascript
ctx.signal.throwIfAborted();
const onAbort = () => { /* 停止本次调用的后续工作 */ };
ctx.signal.addEventListener('abort', onAbort, {once: true});
// 自定义异步步骤结束时移除监听，避免保留不必要的引用。
ctx.signal.removeEventListener('abort', onAbort);
```

运行时仍传递原生取消信号；声明只覆盖 SDK 的取消观察契约。SDK 不引用完整 DOM 库，因此不会为 Node 工程添加 `window`、`document` 等浏览器全局，也不会给同步轻量工程添加 `process`、`Buffer`、`require` 等 Node 全局。新建 Node 工程通过自己的开发依赖和 `jsconfig.json` 显式加载 Node 类型；同步轻量工程保持独立。SDK 刷新不修改作者的 `package.json` 或 `jsconfig.json`。

工程已经提供原生 `AbortSignal` 声明时，`ctx.signal` 沿用该类型，继续兼容需要原生信号的函数；没有环境声明时才使用独立的观察接口。SDK 不会为了取得信号类型自动加入环境库。
