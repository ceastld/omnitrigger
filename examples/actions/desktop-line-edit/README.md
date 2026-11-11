# 删除到行首

`delete-line-start-script` 使用独立轻量宿主，入口为标准 JavaScript `main.js`，不需要 Node。它查询提交时捕获的原目标，发送左 Shift+Home、等待 80ms，再发送 Backspace。`call-script.json` 提供默认调用。

可选参数 `waitMs` 为 0–60000 毫秒整数；`keyDownDelayMs` 为 0–1000 毫秒整数。省略按下延迟时，对 WPS 系列与远程桌面进程使用 10ms，其他进程使用 0ms；显式提供时按调用参数执行。所有参数在发送输入前校验。

这会实际编辑目标文本。原目标失效或不再位于前台时停止，不转向新前台窗口。停止或失败不会撤销先前的选区变化。在行首运行时，Backspace 可能合并上一行；Home 和选择行为由具体编辑器决定，`inputSubmitted` 只表示输入已提交。

本包随附 Node 和 Script SDK 声明，`jsconfig.json` 严格检查两个入口，移出仓库后不再依赖包外类型路径。宿主须包含 `ctx.delay` 能力。

同一包中的 `delete-line-start` 保留 Node 入口 `main.mjs`，调用文件为 `call.json`。

## 作者工程检查

在本目录运行 `npm ci --ignore-scripts`，再执行 `npm run check`。固定 TypeScript 5.9.3 检查两个已有入口及 Node 默认导出，不运行输入配方。环境只加载 ES2022 和宿主 SDK，`types:[]`，不添加 DOM 或 Node 原生全局。

`main.js` 保留旧 Script 函数体协议，仍使用同步 ScriptContext；`main.mjs` 的 JSDoc 使用异步 ActionContext。两个入口原有参数、延时和按键顺序均保留。统一检查可在仓库根运行 `node tools/script-sdk-check/check-project-types.mjs examples/actions/desktop-line-edit`；旧 `.js` 入口会明确记录为 legacy_function_body_protocol，严格类型检查通过不表示模块合同或真实编辑效果通过。

随包声明可用既有 `node tools/script-project.mjs refresh --runtime node --project <本目录>` 显式刷新；刷新保留入口、调用例子、清单、配置和开发依赖，未知 portable 声明仍拒绝覆盖。
