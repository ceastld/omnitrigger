# 复制选中文件

从本次动作提交时的原资源管理器目标取得选中项，逐项检查本地文件或文件夹状态，再一次写入文件剪贴板。Node 与同步 Script 两个入口完成同一个操作。

每次选择 **1–30 项**，保留顺序和重复项。空选区、超量或任一项目不可用时整体失败，不回退当前目录、不静默丢弃项目。同步入口最多使用 32 次宿主 API：一次 Explorer 上下文、最多三十次 stat、一次 writeFiles；不延长宿主时限。

需要匹配版本宿主提供 `ctx.desktop.explorerContext()`、`ctx.fs.stat()` 与 `ctx.clipboard.writeFiles()`。缺写入 API 时在读取 Explorer 前失败。Node 入口使用 default 导出，同步 Script 使用具名 `main(ctx)`。

`writeFiles` 把剪贴板全部格式替换为 Unicode CF_HDROP。`{written:true, sequenceNumber}` 只表示文件列表写入已确认；不表示已粘贴文件，也不保证路径随后仍然有效或剪贴板仍保持该列表。动作不读取原剪贴板、不自动粘贴、剪切或移动文件。

Node 在预检阶段检查取消；写入后才观察到取消时报告 `unknown`。同步调用的取消和时限由宿主处理。两入口均只写一次，保留 SDK 错误的效果分类，不重试、不补发、不恢复剪贴板。

把本目录放到所选配置文件同级的 `actions/copy-selected-files` 下，Node 调用 `call.json`，Script 调用 `call-script.json`。正常动作入口须携带原 Explorer 目标；外部宿主使用 `--run-action <call.json> --target-context <target.json>` 传入新鲜、可核验的目标快照。`--check-action` 只检查引用。

纯检查（仓库根目录）：

```powershell
node --test examples/actions/copy-selected-files/tests/copy-selected-files.test.mjs
```

该检查不证明 Native 写入或实际粘贴通过。真实业务验收还需核对 CF_HDROP 和受控空目标目录中的粘贴结果。此包仅复制显式选中项，不读取文件菜单显示、通配符或排序配置，未实现完整文件菜单。

打开整个动作包目录后，`jsconfig.json` 会严格检查 Node、Script 和共享 helpers，提供本地 SDK 补全。Node 使用异步声明，Script 使用同步 `ScriptMain`；SDK 是随包提供的匹配版本快照，不声明全局 ctx。仓库内可检查实际源码：

```powershell
node tools/script-sdk-check/node_modules/typescript/bin/tsc --project examples/actions/copy-selected-files/jsconfig.json --noEmit
```
