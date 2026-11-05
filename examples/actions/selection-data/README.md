# 选区与剪贴板数据摘要

轻量同步脚本，需支持 `ctx.clipboard.readData()` 和 `ctx.desktop.copySelectionData()` 的应用与脚本宿主。旧宿主不会自动获得新 API。

动作 `examples.selection-data/summarize` 的契约版本为 1，输入必须明确选择：

- `mode: "read"`：读取当前剪贴板的支持格式，不发送复制；不接受 `options`。
- `mode: "copy"`：对提交时捕获且仍为前台的原目标发送一次左 Ctrl+C，再等待序号变化并读取。只调用一次复制接口，不重新抓取目标、不抢焦点、不重试复制。

`copy` 的可选 `options` 直接交给 SDK 严格校验：`timeoutMs` 为 1–5000（默认 600），`pollIntervalMs` 为 1–1000 且不大于超时（默认 15），`settleMs` 为 0–1000（默认 30），`allowUnchanged` 为布尔值（默认 false）。超时是等待序号变化的预算，不是所有系统读取和延迟渲染操作的硬期限。

文本优先，包括空文本；否则返回文件列表。文件路径只作为数据，不检查存在、不枚举、不打开、不转成搜索词。没有支持格式、错误格式或坏 Unicode 会明确报错，不当作空文本。

默认只返回摘要，不记录或输出正文、路径与剪贴板序号：

```json
{"kind":"text","utf16Units":4,"itemCount":1}
```

文件列表的 `utf16Units` 是所有路径字符串的 UTF-16 单元数之和，`itemCount` 为文件项数；合法空列表为 0。文本的 `itemCount` 始终为 1，即使文本为空。`copy` 额外返回 `changed`；`read` 不返回该字段，不能把未执行复制解释为“未变化”。

`allowUnchanged: false` 未观察到变化时返回 `selection_unchanged`。设为 true 才允许读取当时已有的数据，并以 `changed: false` 明示。序号不同不能证明数据一定由原目标产生；等待期间其它程序也可能修改剪贴板。复制结果保留在剪贴板，不恢复旧内容；取消和失焦不会撤销已经发出的复制。

`single_instance: true` 防止同一应用动作服务内该动作跨绑定重叠；不是跨进程全局锁。将整个目录放到配置同级 `actions/selection-data` 后，使用 `call.json` 或 `copy-call.json` 设置动作。实际运行这些调用会读取当时系统数据；示例不包含后台监听、历史采集、调度或个人数据。
