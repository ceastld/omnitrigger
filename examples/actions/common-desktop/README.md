# 常用桌面脚本

两个标准同步 JS 入口都使用本次动作捕获的原目标。它们需要应用提供桌面宿主；独立脚本 CLI 没有目标窗口时会报错。

`close-by-process` 从 `ctrlWProcesses` 读取一个必填的进程名数组。目标名称去掉 `.exe` 后按大小写精确比较：命中时发送左 Ctrl+W，否则发送左 Alt+F4。空数组表示始终发送 Alt+F4。示例调用中的 `Acrobat` 可直接编辑；不读取个人配置或其它程序的动作。

`keyDownDelayMs` 可选，范围 0–1000 毫秒。省略时，对 wps、et、wpp、wpspdf、mstsc 使用 10 毫秒，其余使用 0。脚本先校验全部参数，再读取目标和发送一次组合键。失败直接返回宿主错误，不重试、不继续另一条分支。

`window-action` 直接提交 `toggle_maximize`、`minimize` 或 `close` 窗口请求，使用 `window-call.json` 调用。`close` 是窗口 API 操作；需要按进程选择关闭快捷键时使用 `close-by-process`。

返回的 `submitted` 仅确认系统接纳请求。应用可能弹出保存提示，也可能忽略快捷键，不能据此判断标签页或窗口已关闭。

默认 Alt+F4 分支已通过真实轻量宿主关闭自建 Win32 窗口的[交互验收](../../../docs/DESKTOP_SCRIPT_SMOKE.md)；这不覆盖 Acrobat 的 Ctrl+W 分支或真实文档中的保存提示。

纯脚本检查不会操作真实桌面：

```powershell
node --test examples/actions/common-desktop/recipes.test.mjs
```
