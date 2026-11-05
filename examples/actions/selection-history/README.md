# 单次文本采样到历史

将既有数据读取/复制 API 与独立历史存储组合为一个显式操作；不建立监听、不导入旧历史，不在后台读取系统数据。需要宿主同时提供 `ctx.clipboard.history.append` 和相应采样 API，无接口时先失败，不回退其它来源。

- `{"mode":"read"}`：显式读取一次当时的支持剪贴板格式，只保存文本；不访问或改变原目标。
- `{"mode":"copy","options":{...}}`：向本次提交时捕获且仍为前台的原目标发送一次左 Ctrl+C，再按结果保存文本。options 可省略；不重新抓取前台、不抢焦点、不重复复制。

copy options 只接受既有 SDK 的 timeoutMs（1–5000，默认600）、pollIntervalMs（1–1000且不大于超时，默认15）、settleMs（0–1000，默认30）、allowUnchanged（布尔，默认false）。参数错误先于任何读取/复制/写入。`copy-call.json` 显式示范400/10毫秒等待，不声称与其它采样实现等价。

默认复制未观察到变化则失败，不读取旧文本作为备用。显式允许 allowUnchanged 才可保存 `changed:false` 的文本。changed 只比较序号，不证明数据由原目标生成；超时不是所有 Win32 读取的硬期限。复制结果保留在系统剪贴板；后续保存失败或取消不会撤销已经发出的复制，错误 effectState 会保留先前效果的不确定性。

文本允许空字符串、保留空白，有效 Unicode、无 NUL、最多65536 UTF-16单元。文件列表即使为空也返回 `history_text_required`，不拼接路径、不查文件、不转成搜索词。没有支持格式和损坏结果明确失败。

历史写入一次，不重试；连续重复由 store 返回 inserted:false。终态只有操作、saved/inserted、长度及 copy 的 changed，没有正文、路径、序号、ID或revision。错误消息固定，不回显内容。两个入口分别 single_instance，不宣称跨动作互锁，存储负责并发。

```powershell
node --test examples/actions/selection-history/recipes.test.mjs
```

测试只用合成 RecordingHost。实际运行调用会访问当时剪贴板或发送复制，并写入应用指定的历史；示例不会自动安装绑定或启动监听。真实原目标、取消与存储验收需另行完成。
