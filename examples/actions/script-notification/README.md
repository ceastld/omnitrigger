# 脚本通知示例

此工程提供同一通知的 Node 异步入口 `main.mjs` 和轻量同步入口 `main.js`。编辑 `call.json` 或 `call-script.json` 中的 `input` 即可设置正文、标题、级别、位置与显示时长；两个入口均调用共享契约 `ctx.ui.notify(options)`，返回 `{ "submitted": true }`。

`message` 必填，为 1–1024 个 UTF-16 单元的非空白有效 Unicode 文本，允许换行、回车和 Tab，其余控制字符拒绝。`title` 默认 `OmniTrigger`，为最多 128 个 UTF-16 单元的非空白单行文本。`level` 支持 `info`、`success`、`warning`、`error`，默认 `info`。`placement` 支持主显示器工作区的 `topRight`、`bottomRight`、`center`，默认 `topRight`。`durationMs` 为 1000–30000 的整数，默认 3000。参数不会自动裁剪或强制转换，未知字段会被拒绝。

需要运行中的 OmniTrigger 应用界面。没有 UI 宿主时返回 `ui_unavailable`，已有通知占用名额时返回 `ui_busy`；不会启动另一个应用、静默排队或覆盖已有通知。错误带有 `method: "ui.notify"` 与 `effectState`；后者为 `unknown` 时，不能据此断定没有显示或自动重试。

通知只显示纯文本，不执行 HTML 或链接；窗口不激活、不抢前台。Node 必须 `await`，轻量入口同步取得创建回执。回执确认应用已接纳并创建通知界面请求，不证明用户看见、读到或完成业务；返回不等待显示时长。脚本正常完成后，通知仍按时长存活，应用退出时统一收尾。

窗口大小受主显示器工作区限制，过长内容可能被裁切，适合简短提示。示例不创建定时任务，不改触发绑定、剪贴板或系统通知设置。显示位置与计时效果需要在应用中验证，内存宿主测试只能验证参数、回执与错误契约。
