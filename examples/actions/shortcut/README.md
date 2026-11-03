# 发送快捷键

把本目录放入配置同级 `actions` 目录，使用 `call.json` 的结构调用。`keys` 和 `timing` 与原生 `send_shortcut` 动作共用格式；脚本通过 `ctx.desktop.sendShortcut(keys, {timing})` 复用 Rust 的进程时序解析。目标在提交时捕获，取消或切窗不会另选窗口。

返回 `{submitted:true}` 只表示输入被系统接纳。标签切换的实际效果可用隔离 Chrome 验收：在 `scripts/test-browser-actions.mjs` 原有连接器、扩展、Playwright、Chrome 和 `--app` 参数后添加 `--desktop-shortcuts examples/actions/shortcut/smoke-cases.json`。该选项打开有界面的独立测试 profile，创建三个本地页面；调用前核验本次 Chrome 的进程 ID、唯一标题和前台状态，再分别检查上一标签、下一标签和关闭当前标签的结果。测试只操作新建窗口，结束后关闭隔离 profile。

侧键绑定示例见 [`mouse-tab-navigation.json`](../../mouse-tab-navigation.json)。三条绑定初始停用，前台与指针窗口均限制在指定浏览器；上滚为右 Ctrl+右 Shift+Tab，下滚为右 Ctrl+Tab，右键为右 Ctrl+W。滚轮每个非零事件触发一次，未命中动作的侧键单击仍按引擎的回退规则交还。启用前先核对已有同组合绑定；示例本身不会更改当前配置。
