# 脚本动作工程

编辑 `main.js`，在 `call.json` 中填写输入。入口为标准 JavaScript 函数 `main(ctx)`，返回 JSON 值，无需转译或 npm 安装。SDK 类型声明位于 `sdk/api.d.ts`，`jsconfig.json` 开启 JS 类型检查；使用支持 JavaScript / TypeScript 的编辑器即可获得补全。

工程已放入配置同级的 `actions` 目录。使用相同配置调用 OmniTrigger 的 `--check-action` 或 `--run-action`，参数为本工程 `call.json` 的路径。`--check-action` 校验引用和运行时是否可用，不执行或静态检查脚本；`--run-action` 才执行入口。应用已经打开时，需要重新启动应用才能在动作库看到新工程。

执行需要独立脚本宿主；开发时将宿主放在配置同级 `runtimes/script/host.exe`，或使用 `OMNITRIGGER_SCRIPT_HOST` 指定完整路径。创建工程本身不下载宿主。

`ctx.fs`、`ctx.process`、`ctx.clipboard`、`ctx.open`、`ctx.desktop` 和 `ctx.browser` 为同步 API，不使用 `await`。桌面 API 使用提交动作时的原目标。浏览器调用复用已安装并启用的浏览器连接器，支持 `call`、`profiles` 与 `chrome.*`；当前没有 Node / npm 模块、ESM 导入或 Promise 入口。示例只处理输入并返回结果，不操作窗口或剪贴板。

SDK 是创建时附带的类型快照，更新应用不会覆盖本工程。它提供补全和静态参数检查，不代替实际运行时校验；文件或系统操作可能产生副作用，失败后不会自动撤销或重试。
