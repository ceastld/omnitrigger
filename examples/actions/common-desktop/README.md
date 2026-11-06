# 常用桌面脚本

四个标准同步 JS 入口都使用本次动作捕获的原目标。它们需要应用提供桌面宿主；独立脚本 CLI 没有目标窗口时会报错。

`close-by-process` 从 `ctrlWProcesses` 读取一个必填的进程名数组。目标名称去掉 `.exe` 后按大小写精确比较：命中时发送左 Ctrl+W，否则发送左 Alt+F4。空数组表示始终发送 Alt+F4。示例调用中的 `Acrobat` 可直接编辑；不读取个人配置或其它程序的动作。

`keyDownDelayMs` 可选，范围 0–1000 毫秒。省略时，对 wps、et、wpp、wpspdf、mstsc 使用 10 毫秒，其余使用 0。脚本先校验全部参数，再读取目标和发送一次组合键。失败直接返回宿主错误，不重试、不继续另一条分支。

`window-action` 直接提交 `toggle_maximize`、`minimize` 或 `close` 窗口请求，使用 `window-call.json` 调用。`close` 是窗口 API 操作；需要按进程选择关闭快捷键时使用 `close-by-process`。

返回的 `submitted` 仅确认系统接纳请求。应用可能弹出保存提示，也可能忽略快捷键，不能据此判断标签页或窗口已关闭。

`send-win-a` 使用空对象参数，向原目标提交一次左 Win+A，调用示例为 `win-a-call.json`。按下延迟为 0 毫秒，对 wps、et、wpp、wpspdf、mstsc 使用 10 毫秒，由原生 API 根据捕获目标解析。`inputSubmitted:true` 仅确认输入提交；具体系统面板及切换效果由 Windows 版本、策略和当前状态决定，尚未完成真实面板效果验收。回执不明确时报告 `input_unconfirmed`、效果未知，不重试，也不调整面板内的设置。

`paste-arrow-text` 接受必填 `text`、`pasteKey` 和 `keyDownDelayMs`。文本仅允许字面 `\Leftarrow ` 或 `\Rightarrow `，末尾空格保留；粘贴键明确选择 `ctrl-v` 或 `shift-insert`，按下延迟为 0–1000 毫秒整数。全部参数校验及原目标前台检查均在写入剪贴板之前完成。

`paste-left-arrow.json`、`paste-right-arrow.json` 提供两份显式调用，默认示例值是 Ctrl+V、0 毫秒；需要其它设置时编辑这两个字段。动作写入剪贴板后固定等待 50 毫秒，再向原目标发送一次粘贴组合键。它不读取或恢复旧剪贴板、不补 Enter、不重试，也不自动读取其它程序的粘贴策略。该等待不包含剪贴板序号轮询，整段操作不是原子事务。

成功返回 `clipboardWritten` 和 `pasteInputSubmitted`，后者仅确认系统接纳组合键。写入成功后若等待或粘贴失败，报 `clipboard_written_paste_unconfirmed`、效果状态 `unknown`；错误信息明确剪贴板已经写入，不能据此假定整段没有副作用。目标应用最终效果仍需单独验证。

默认 Alt+F4 分支已通过真实轻量宿主关闭自建 Win32 窗口的[交互验收](../../../docs/DESKTOP_SCRIPT_SMOKE.md)；这不覆盖 Acrobat 的 Ctrl+W 分支或真实文档中的保存提示。

纯脚本检查不会操作真实桌面：

```powershell
node --test examples/actions/common-desktop/recipes.test.mjs
node --test examples/actions/common-desktop/paste-arrow.test.mjs
node --test examples/actions/common-desktop/system-panel.test.mjs
```
