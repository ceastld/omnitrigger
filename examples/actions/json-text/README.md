# JSON 文本排版与子值选择

处理调用参数明确提供的 JSON 文本，提供 Node 和同步轻量模块入口。

| 动作 ID | 运行时 | 用途 |
| --- | --- | --- |
| `inspect-or-send` | Node | 校验、排版或选择排版后输入 |
| `inspect-or-send-script` | script | 同步校验、排版或选择排版后输入 |
| `select-or-send` | Node | 按明确路径选取 JSON 子值 |
| `select-or-send-script` | script | 同步选取 JSON 子值 |

四个动作的契约版本均为 1，分别启用单实例；不同动作 ID 不互锁。

## 校验与排版参数

- `{mode:"inspect",text}`：校验并返回根类型、节点/键/字符串/数字数量、容器深度及输入长度，无宿主调用。
- `{mode:"send",text,format:"pretty"|"compact"}`：完整校验、排版后，向提交时捕获的原目标输入一次。
- `{mode:"choose-send",text}`：先验证两种排版结果，再选择缩进或紧凑格式；取消不发送。

仅接受对应模式列出的字段。缩进使用两格和 LF；紧凑格式移除字符串外的 JSON 空白。大整数、负零、指数写法、属性顺序和字符串转义保持原 token，数字不转换为 JS number。

## 子值选择参数

两个选择动作均只接受 `mode`、`text`、`path`、`format`：

| 参数 | 规则 |
| --- | --- |
| `mode` | `inspect` 返回摘要；`send` 向原目标输入所选 JSON 文本一次 |
| `text` | 完整 JSON 文本 |
| `path` | 最多 32 段的稠密数组；字符串为精确对象键，整数为数组索引；空数组选择根 |
| `format` | `compact` 或 `pretty` |

例如 `text` 为 `{"items":[{"count":9007199254740993}]}`，`path` 为 `["items",0,"count"]`，选中 JSON 文本保持为 `9007199254740993`。字符串子值保留 JSON 引号和转义。

键名按解码后精确匹配，可使用空键；每个键最多 1024 个 UTF-16 单元，须有效 Unicode、无 NUL。数组索引为 0–9999 的整数，不把字符串数字当索引。路径不支持通配、递归搜索或表达式，也不采用 JSON Pointer 字符串语法。缺键/索引报 `json_path_missing`，节点或路径段类型不匹配报 `json_path_type`。

## 限额与结果

输入最多 65,536 个 UTF-16 单元、32 层容器和 10,000 个值节点；要求有效 Unicode、无实际 NUL。不接受注释、BOM、尾逗号或多份文档，重复对象键（含转义后同名）明确拒绝。整份文档均须有效，选择路径不能绕过其它分支的校验。JSON 转义中的 `\u0000` 等保留为文本。

输出最多 65,536 个 UTF-16 单元；缩进膨胀超限时报错，不截断或自动换格式。选择动作摘要包含子值类型、子树节点数、路径段数及长度；终态和错误不包含正文、键名或路径。`inputSubmitted:true` 表示系统接纳输入请求，失败/取消/失焦后不自动重试，不改抓其它目标。

默认调用文件均为无宿主调用的 `inspect`。`send-call.json` 和选择动作的 `mode:"send"` 会真实输入文本。包不自动读取剪贴板、选区或文件。

## 编辑与静态检查

用支持 JavaScript/TypeScript 的编辑器打开整个包目录。`jsconfig.json` 为所有七个业务模块启用 strict `checkJs`，测试文件不属于作者检查入口。

- `sdk/api.d.ts` 是本地 Node API 声明。
- `sdk/script-types.d.ts` 是本地同步模块 API 声明，没有全局 `ctx`。
- `json-types.d.ts` 描述输入、路径、任务、摘要和结构化错误；入口的外部输入保留为 `unknown`，由现有校验器检查。

共享模块显式接收上下文，JSDoc 和声明引用均为包内路径。更新 SDK 时先保留作者原快照，再选择与 App/宿主匹配的声明；替换声明不会升级实际运行时。

使用项目已有检查工具，路径按本机实际位置填写：

```powershell
$checkTools = 'D:\OmniTrigger\tools\script-sdk-check'
$package = 'D:\Actions\json-text'
node "$checkTools\node_modules\typescript\lib\tsc.js" --project "$package\jsconfig.json" --noEmit
node "$checkTools\check-project-types.mjs" $package
```

第一条检查所有 Node/同步业务模块；第二条额外检查清单中的同步入口、静态模块图和 `main` 导出。静态通过不代表目标窗口或运行时能力已经验收。
