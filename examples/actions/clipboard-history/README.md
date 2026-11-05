# 显式文本历史

本包是依赖历史 API 的独立配方，需应用和宿主接入 `ctx.clipboard.history` 后才能使用；仅把包放入目录不会自动获得持久化服务。无桥接时明确返回 `history_unavailable`，不改读系统剪贴板，不创建监听器。

Node 异步入口为 `manage`，轻量同步模块入口为 `manage-script`。两者共用 `workflow.mjs`，动作契约版本均为 1，各自声明 `single_instance: true`。两个不同动作 ID 不互锁；历史存储仍须负责并发与取消保护。

动作 `open`（Node）或 `open-script`（轻量同步）无需输入参数，通过 `ctx.ui.openHistory()` 在已有主进程打开完整独立历史页。调用文件为 [open-call.json](open-call.json) 和 [open-script-call.json](open-script-call.json)，不需要填写程序或配置路径。目标在本次动作提交时捕获，窗口打开后不会改抓前台；默认关闭采集，也不启动第二套 Hook。创建回执返回后窗口继续存活。关闭时只停止该窗自己的采集，确认停止后才移除窗口；停止失败保留窗口供重试，不退出主程序或停止主窗采集。已有选择窗或 API 历史窗时返回 `ui_busy`；无应用界面的 CLI 返回 `ui_unavailable`。回执仅确认窗口创建，取消或结果不确定时不自动重试。应用、宿主和 SDK 需支持该 API。

管理入口的输入必须明确选择一种模式，不接受其它参数：

- `{"mode":"append","text":"原样保存的文本"}`：显式保存输入，不读取剪贴板。允许空文本与空白，最多 65536 UTF-16 单元，无 NUL 或坏代理项。连续重复由存储返回 `inserted:false`。
- `{"mode":"list","limit":100}`：仅查询摘要并显示选择框；选中后只返回摘要，不读取全文、不发送输入。
- `{"mode":"send","limit":100}`：列出摘要，用户选择后按稳定 ID 读取该条全文，再调用一次原目标 `sendText`。空文本返回 `empty_text`，不发送输入。

`list` / `send` 的 `limit` 可省略，默认 100，范围 1–100，给选择框请求大小留出余量。空历史直接返回 `empty`，不显示空选择框；关闭或超时返回 `cancelled`。条目在选择期间被删除时明确失败，不重选、不回退其它条目，不重试发送。

选择框只显示单行摘要。空白摘要使用占位标签；标签中的控制字符替换为空格，取出的正文不会 trim、截断、转义或替换。`sendText` 直接发送 Unicode，不改写剪贴板、不注入 Ctrl+V、不重新抓取前台。选择框关闭后的目标校验与焦点恢复由宿主处理，失败即停止；系统接纳输入不等于业务已完成，部分输入不能撤销。

终态仅返回操作、状态、数量、UTF-16 长度及 `inputSubmitted` 或 `inserted`；不返回正文、摘要、稳定 ID、时间或 revision。异常消息固定，未知错误代码归为 `history_action_failed`，不回显宿主正文，也不自动重试副作用。

纯内存 RecordingHost 测试：

```powershell
node --test examples/actions/clipboard-history/recipes.test.mjs
```

测试不会读取系统剪贴板、启动监听器、访问文件路径或发送真实输入。示例调用文件供开发检查；实际运行 `append` 会写入应用配置的历史存储，运行 `list` 会展示摘要，运行 `send` 会向原目标提交输入。
