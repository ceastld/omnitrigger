# 显式文本行整理

只处理调用方明确传入的 `text`，提供 Node 异步和轻量同步 ESM 入口。没有选区、剪贴板、文件或进程读取，不推测输入来自哪个编辑器。

| `operation` | 固定规则 |
| --- | --- |
| `deduplicate` | 按行完整字符串精确去重，保留第一次出现的顺序；空行也参与去重。 |
| `sort` | 按 JavaScript UTF-16 码元字典序升序排序，保留重复行。与语言区域排序、自然数字排序不同。 |

两种操作均识别 CRLF、LF 和单独 CR，输出统一为 LF。保留输入是否有终止换行；终止换行本身不额外计为空行。空文本为零行，单个换行为一个空行。不会 trim、折叠空格、忽略大小写或做 Unicode 归一化；例如 `a`、`A` 和 ` a` 是不同的行。

只接受 `mode`、`operation`、`text` 三个字段，均须明确提供。输入最多 65,536 个 UTF-16 单元、10,000 行，要求有效 Unicode、无 NUL；输出也有 65,536 单元上限，不截断。

- `mode:"inspect"`：完成变换与校验，只返回行数、长度、移除行数、是否变化和是否保留终止换行，没有宿主调用。
- `mode:"send"`：先完整变换，再向提交时捕获的原目标 `sendText` 一次。空输出返回 `status:"empty"`，不发送；仅含换行的输出仍会发送。

终态和错误不包含输入行或结果正文。发送失败、失焦或取消后不重试，不补抓前台，不使用剪贴板回退。`inputSubmitted:true` 只代表系统接纳输入，不能保证业务完成或撤销部分输入。两个动作 ID 分别启用单实例，不互锁。

默认调用均为无宿主调用的 inspect，`send-call.json` 实际运行会输入文本。本包提供明确的行变换规则，不提供文本菜单界面或自动取选区。

```powershell
node --test examples/actions/line-text/recipes.test.mjs
```

测试只使用合成文本与 RecordingHost。


## 作者工程检查

在本目录运行 `npm ci --ignore-scripts`，再执行 `npm run check`。固定 TypeScript 5.9.3 严格检查两个入口、共享流程、行变换和 Node 默认导出；不运行去重、排序或输入动作。随包 SDK 与 JSDoc 均为本地引用，可移出仓库使用。

配置只加载 ES2022 和显式宿主声明，types:[]、strict/noEmit、skipLibCheck:false；不提供 DOM/Node 原生全局。NodeContext 使用异步回执，SyncContext 使用同步回执；外部 input 保持 unknown，由原有校验处理。类型断言连接既有校验和声明，不代替运行时参数检查。

仓库根可用 `node tools/script-sdk-check/check-project-types.mjs examples/actions/line-text` 检查模块合同。既有 `node tools/script-project.mjs refresh --runtime node --project <本目录>` 显式刷新整组 SDK；入口、算法、清单、调用、配置和开发依赖保留，未知 portable 声明仍拒绝覆盖。类型通过不表示输入效果已验收。
