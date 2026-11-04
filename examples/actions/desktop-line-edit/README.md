# 删除到行首

`delete-line-start-script` 使用独立轻量宿主，入口为标准 JavaScript `main.js`，不需要 Node。它查询提交时捕获的原目标，发送左 Shift+Home、等待 80ms，再发送 Backspace。`call-script.json` 提供默认调用。

可选参数 `waitMs` 为 0–60000 毫秒整数；`keyDownDelayMs` 为 0–1000 毫秒整数。省略按下延迟时，对 WPS 系列与远程桌面进程使用 10ms，其他进程使用 0ms；显式提供时按调用参数执行。所有参数在发送输入前校验。

这会实际编辑目标文本。原目标失效或不再位于前台时停止，不转向新前台窗口。停止或失败不会撤销先前的选区变化。在行首运行时，Backspace 可能合并上一行；Home 和选择行为由具体编辑器决定，`inputSubmitted` 只表示输入已提交。

在仓库中打开此目录即可使用 `jsconfig.json` 类型检查。移到仓库外时，先在应用中创建脚本工程，再移入 `main.js` 的函数逻辑，保留新工程的本地 SDK 引用。宿主须包含 `ctx.delay` 能力。

同一包中的 `delete-line-start` 保留 Node 入口 `main.mjs`，调用文件为 `call.json`。
