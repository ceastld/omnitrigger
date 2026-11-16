# 包内 JavaScript 模块

轻量脚本宿主使用引擎原生 ESM，不转译源码，不依赖 Node、npm、打包器或自定义文件语法。新工程采用 `main.mjs`，具名导出函数 `main(ctx)`（同步模板如下）：

```js
import { cleanText } from './lib/text.js';
/** @type {ScriptMain} */
export function main(ctx) {
    return { text: cleanText(ctx.input.text) };
}
```

`lib/text.js` 使用普通 `export`，不隐式获得 `ctx`；需要 SDK 时将上下文或所需能力显式传入。入口仍接受同一份 SDK，入口可返回 JSON 值或其 Promise，最终结果仍必须是 JSON。必须具名导出 `main`；仅默认导出、缺少具名 `main` 或该导出不是函数时返回 `module_entry`。

## 入口及协议兼容

动作清单的 `runtime` 仍为 `script`。`.mjs` 入口按模块执行；既有 `.js` 入口保留 `function main(ctx)` 约定，由应用适配层调用该函数。不会自动改写作者文件、猜测源码是否使用 `import`，也不会根据 `package.json` 改变解释。

独立宿主继续接受版本 1 JSON 请求。原有函数体请求保持有效：

```json
{"script":"return ctx.input;","input":{"text":"example"}}
```

模块请求改用 `module_entry`，相对于绝对本地 `package_root`，入口扩展名须为 `.mjs`：

```json
{"module_entry":"main.mjs","package_root":"D:/actions/example","input":{"text":"example"}}
```

`script` 与 `module_entry` 互斥，二者都缺少时拒绝。`invocation`、`native_context` 和响应信封保持不变；调用者无需在 stdin 中发送模块源码。旧宿主不认识 `module_entry`，使用新工程前需要安装包含本实现的宿主。

## 路径与读取边界

- 仅接受 `./` 或 `../` 开头、仍位于动作包内的导入，必须显式写出 `.js` / `.mjs`。按导入者所在目录解析，归一化 `.` / `..`，按规范化文件路径缓存。
- 不做扩展名或 `index.js` 补全，不加载裸包名、Node 内置模块、绝对路径、URL、反斜杠、百分号转义、查询串、fragment、JSON、DLL 或导入属性。
- 在打开文件前逐级检查路径属性，拒绝链接、联接点、云盘目录及 Offline / RecallOnOpen / RecallOnDataAccess 占位文件；Windows 下拒绝 UNC、设备名、备用数据流及不明确的尾部字符。
- 每次调用最多读取 64 个模块；每个文件最多 128 KiB，累计最多 1 MiB，包含入口。仅读取实际导入的文件，不扫描目录。需要 UTF-8 源码和可表示为 UTF-8 的模块路径。

这是加载范围和资源边界，不是安全沙盒。与现有包文件检查一样，属性检查与打开之间仍存在本机并发替换窗口；既有 SDK 权限与当前用户权限不变。

## 循环、缓存与初始化

两个引擎各自使用原生模块记录缓存，不自行执行依赖排序。相同规范化文件在一次调用中解析及初始化一次；循环导入和实时绑定遵循 ESM，循环中提前访问未初始化绑定产生原生异常。文件快照只活到本次宿主进程退出，下次调用重新读取。保存源码即可让下一次执行使用新内容。

模块初始化推进引擎内部任务队列，无法完成的初始化 Promise 返回 `module_pending`；宿主没有外部异步事件循环。工程采用静态 `import` / `export`，入口可为 async 或返回 Promise。新版运行器等待入口结果，并推进引擎任务和已配置的异步浏览器通道；不把动态导入、顶层异步初始化或未被入口等待的后台任务作为动作完成协议。

## 错误与源文件定位

模块边界错误使用对象，包含 `code`、中文 `message`、`effect_state`：`module_entry`（入口契约）、`module_specifier`（路径形式）、`module_path`（包边界）、`module_nonlocal`（链接或占位）、`module_unavailable`（读取或 UTF-8）、`module_parse`（语法）、`module_limit`（数量或大小）、`module_pending`（初始化未完成）。解析和加载阶段的故障通常为 `not_started`；初始化已开始后发生的加载故障与未完成初始化保守标为 `unknown`。不自动重试，也不声称撤销已发生的副作用。

引擎直接接收每个源文件的真实路径，不拼接依赖文件或改变其行号。依赖语法错误报告文件路径及引擎诊断，运行异常的调用栈可定位辅助文件；原生 API 的 `code`、`effect_state`、`method`、`required_permissions` 继续保留。引擎对语法、限额和异步拒绝的栈细节可能不同；不提供源映射。消息继续执行既有 UTF-8 / JSON 编码长度截断。

## 编辑器与核对依据

工程附带全目录 `.js` / `.mjs` 类型检查配置（ESNext 模块、Bundler 解析模式）及本地 SDK 快照，不只检查入口文件。VS Code 打开文件夹及入口；`OMNITRIGGER_EDITOR` 指定 Code、Code - Insiders、Cursor、Codium 或 Windsurf EXE 时也传入工程文件夹。其他显式编辑器保留仅文件参数，不推断未知编辑器的命令行协议。

实现核对了当前锁定源码：Boa **0.21.1** 的 `ModuleLoader::load_imported_module`、`Module::parse/load/link/evaluate/get_value` 及 `Source::with_path`；rquickjs **0.14.0** 的 `Resolver` / `Loader`（含 `ImportAttributes` 参数）、`Runtime::set_loader`、`Module::declare/eval/get` 和 `Promise::finish`。QuickJS 路线需启用现有依赖的 `loader` 特性；不启用 `dyn-load` 或 Rust 异步适配。`loader` 使用已在独立锁文件中的 relative-path 2.0.3（MIT OR Apache-2.0，选择 MIT）。可参考 [Boa 0.21.1 模块接口](https://docs.rs/boa_engine/0.21.1/boa_engine/module/index.html) 和 [rquickjs 0.14.0 特性说明](https://docs.rs/rquickjs/0.14.0/rquickjs/#advanced)；不要按 `latest` 页面实现，因为 Boa 新版的 trait 签名已经不同。

`tests/modules.rs` 提供两引擎共用的命令行回归，覆盖真实多文件导入、再导出、规范化缓存、循环及实时绑定、跨调用重载、边界拒绝、缺失/损坏/超限文件、入口契约、初始化未完成、辅助文件诊断和原生错误字段。应用适配与工程生成的定向单元测试分别位于 `script_host.rs`、`project.rs`；编辑器参数测试位于 `script_projects.rs`。

同步函数或返回 Promise 的普通函数使用 `ScriptMain`。直接声明 `async function` 时使用 `/** @type {ScriptAsyncMain} */`：TypeScript 的 JSDoc 要求 async 函数的返回注释直接为 Promise，不能使用 JSON 与 Promise 的联合返回注释。两者使用相同的上下文，最终结果都必须是 JSON。
