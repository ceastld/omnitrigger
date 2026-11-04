# 脚本动作工程

编辑 `main.js`，在 `call.json` 中填写输入。入口为标准 JavaScript 函数 `main(ctx)`，返回 JSON 值，无需转译或 npm 安装。SDK 类型声明位于 `sdk/api.d.ts`，`jsconfig.json` 开启 JS 类型检查；使用支持 JavaScript / TypeScript 的编辑器即可获得补全。保留入口上方的 `@type {ScriptMain}` 注释，编辑器会推断 `ctx` 类型，并检查每条返回路径；误用 `async`、遗漏返回或返回 Promise、函数、`undefined` 等内容会报告类型错误。没有业务结果时可返回 `null`。

工程已放入配置同级的 `actions` 目录。应用中可用“编辑代码”启动本地编辑器，保存后下次运行读取新代码；动作库的“重新加载”更新新增包、动作信息和运行时，不会停止已经运行的调用。创建工程不自动添加触发绑定。

开发时可使用相同配置调用 OmniTrigger 的 `--check-action` 或 `--run-action`，参数为本工程 `call.json` 的路径。`--check-action` 校验引用和运行时是否可用，不执行或静态检查脚本；`--run-action` 才执行入口。

执行需要独立脚本宿主；开发时将宿主放在配置同级 `runtimes/script/host.exe`，或使用 `OMNITRIGGER_SCRIPT_HOST` 指定完整路径。创建工程本身不下载宿主。

`ctx.fs`、`ctx.process`、`ctx.clipboard`、`ctx.open`、`ctx.desktop` 和 `ctx.browser` 为同步 API，不使用 `await`。桌面 API 使用提交动作时的原目标。浏览器调用复用已安装并启用的浏览器连接器，支持 `call`、`profiles` 与 `chrome.*`；当前没有 Node / npm 模块、ESM 导入或 Promise 入口。示例只处理输入并返回结果，不操作窗口或剪贴板。

SDK 是创建时附带的类型快照，更新应用不会覆盖本工程。它提供补全和静态参数检查，不代替实际运行时校验：`any`、循环引用和非有限数等仍需作者检查；文件或系统操作可能产生副作用，失败后不会自动撤销或重试。
