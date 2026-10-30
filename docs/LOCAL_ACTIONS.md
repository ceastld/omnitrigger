# 本地步骤动作

本地步骤动作把常用的按键、文字、剪贴板和打开操作按顺序组合，在 OmniTrigger 后台执行。触发器和轮盘槽位共用同一份动作格式，无需为这些基础操作启动外部解释器。

## 在界面中配置

从“动作库”的本地能力入口创建草稿，或在触发器详情、轮盘槽位中选择“本地步骤动作”。使用小表单添加步骤、填写参数，并通过上移、下移、删除调整顺序。连续组合键分开填写整组保持的修饰键和逗号分隔的主键，例如修饰键 `Ctrl`、主键 `K, Z`。

保存只验证和更新配置，不执行步骤。“运行当前草稿”和实际触发会真实发送输入、读写剪贴板或打开目标。界面显示当前步骤、完成、失败或取消；触发器中的“停止此动作”、轮盘中的“停止此本地动作”以及 Esc 只阻止后续步骤，已经完成的操作不会回滚。

## 支持的步骤

| 步骤 | 配置字段 | 行为 |
|---|---|---|
| `send_shortcut` | `keys` | 发送一次普通键或修饰键组合，例如 Ctrl+C |
| `key_tap` | `key` | 单独点按一个键，也支持单独 Alt、Ctrl、Shift、Win |
| `key_sequence` | `modifiers`、`keys` | 修饰键在整组主键之间持续按住，主键按顺序分别按下、释放 |
| `input_text` | `text` | 发送 Unicode 文本，不借用剪贴板；换行转换为回车输入 |
| `set_clipboard` | `text` | 写入 Windows 文本剪贴板；原剪贴板格式会被替换 |
| `read_clipboard` | `variable` | 读取文本剪贴板到本次运行变量 |
| `set_variable` | `name`、`value` | 设置本次运行变量，可使用前面定义的变量 |
| `open` | `target` | 通过系统关联打开网址、本地文件或文件夹 |
| `delay` | `milliseconds` | 等待指定毫秒数，等待期间仍检查取消与原前台身份 |
| `copy_selection` | `variable` | 向原窗口发送 Ctrl+C，等待剪贴板变化后读取文本；复制结果留在剪贴板，600ms 内未复制或不含文本时失败 |
| `read_window_title` | `variable`、`strip_app_name` | 读取原窗口标题；`strip_app_name` 为 `true` 时去掉末尾的“ - 应用名” |
| `open_urls` | `text` | 按出现顺序打开文本中的 http(s) 与 `www.` 网址，去重后最多 8 个；没有网址时失败 |
| `web_search` | `url`、`query` | 去掉首尾空白后取查询前 128 个字符，UTF-8 编码后替换 `url` 中唯一的 `%s` 并打开 |

按键字段保存 Windows 虚拟键码。`key_sequence` 的 `keys` 允许重复，例如 `[72, 72]` 表示连续点按两次 H；修饰键为空时发送普通键序列。连续组合键的一组最多 16 个主键，最多四类左右明确的修饰键；无效键、重复修饰键或将修饰键填入主键序列会拒绝保存。

文本可引用 `${name}`，`$$` 表示字面量 `$`。变量按步骤顺序定义，只在本次运行内有效，最多 16 个；变量名为 1–32 个 ASCII 字母、数字或下划线，首位不能为数字。变量值不递归解释，也不读取环境变量。读取剪贴板和复制选中文本只保存文本，不提供图片或文件列表。

`open` 接受 `http`、`https`、`mailto`、`about` 地址，以及盘符开头的本地绝对路径。网址中的空白需要编码；不接受相对路径或任意自定义协议。系统关联决定具体打开程序，浏览器的当前标签、新标签和新窗口行为不由本步骤控制。需要指定程序、参数和工作目录时使用独立的“运行程序”动作。

## JSON 示例

以下动作按住左 Ctrl，依次点按 K、Z，最后释放本次按下的 Ctrl：

```json
{
  "kind": "local",
  "version": 1,
  "steps": [
    { "kind": "key_sequence", "modifiers": [162], "keys": [75, 90] }
  ]
}
```

单独点按 Alt，等待菜单响应，再输入两个普通键：

```json
{
  "kind": "local",
  "version": 1,
  "steps": [
    { "kind": "key_tap", "key": 164 },
    { "kind": "delay", "milliseconds": 100 },
    { "kind": "key_sequence", "modifiers": [], "keys": [72, 49] }
  ]
}
```

读取文本剪贴板，并在其前面加上固定文字后输入：

```json
{
  "kind": "local",
  "version": 1,
  "steps": [
    { "kind": "read_clipboard", "variable": "clip" },
    { "kind": "input_text", "text": "内容：${clip}" }
  ]
}
```

复制选中文本，用 Google 搜索：

```json
{
  "kind": "local",
  "version": 1,
  "steps": [
    { "kind": "copy_selection", "variable": "text" },
    { "kind": "web_search", "url": "https://www.google.com/search?q=%s", "query": "${text}" }
  ]
}
```

在 Chrome 中复制当前网址与标题，组成 Markdown 链接写入剪贴板：

```json
{
  "kind": "local",
  "version": 1,
  "steps": [
    { "kind": "send_shortcut", "keys": [162, 76] },
    { "kind": "delay", "milliseconds": 80 },
    { "kind": "copy_selection", "variable": "url" },
    { "kind": "key_tap", "key": 27 },
    { "kind": "read_window_title", "variable": "title", "strip_app_name": true },
    { "kind": "set_clipboard", "text": "[${title}](${url})" }
  ]
}
```

## 执行与保存边界

每个动作需要 1–32 步，单次显式等待最多 5000ms，显式等待总和最多 10000ms。文本与模板展开结果最多 65,536 个 UTF-16 单元，整个本地动作配置最多 256 KiB。未知字段、未知步骤类型和不支持的版本会报错。

动作提交时冻结原前台窗口标识，每步开始和实际调用能力前再次检查；输入适配还会在提交过程中检查目标。切换前台、目标失效、取消或任一步失败都会停止后续步骤，不抢回焦点，也不自动重试。打开目标可能合法切换前台；若后面还有步骤，会继续按原前台窗口检查。`open_urls` 打开第一个网址后不再要求原窗口在前台，只检查取消。将打开操作安排在最后，通常更符合这项语义。

本地步骤在 Hook 之外由独立后台线程顺序执行，总计最多四个排队、运行中或终态尚未读取的任务；同一绑定运行期间拒绝重复提交。输入仍使用统一的注入标记与发送约束，权限隔离和其他输入工具可能造成失败或部分提交。失败或取消不会撤销已输入文字、已修改的剪贴板或已打开的目标，记录中的完成步骤数也不能证明目标软件完成了对应业务。

绑定配置读取版本 1–8、保存版本 8；轮盘配置读取版本 1–3、保存版本 3；本地动作内部版本为 1。首次保存旧配置会保留对应版本的 `.bak` 备份。可以使用 `--check-config` 校验格式、步骤参数和引用，该命令不执行动作。当前版本同时支持[通用 JS 动作包](ACTION_PACKAGES.md)，本地步骤配置保持原有结构。

首版没有内嵌 JavaScript、TypeScript、Node 或其他脚本语言，也不是通用脚本沙盒。普通“运行程序”动作仍可明确调用已安装的解释器来运行脚本，参见[外部动作与脚本示例](COMMAND_ACTIONS.md)。按键发送约束见[发送按键与快捷键](KEYBOARD_ACTIONS.md)。
