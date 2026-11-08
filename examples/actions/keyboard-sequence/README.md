# 有序键盘与文本步骤

`recipe-script` 使用独立轻量宿主，不需要 Node。入口 [recipe.js](recipe.js) 是标准同步 `function main(ctx)`，直接组合现有的 `ctx.desktop.sendKeySequence`、`ctx.desktop.sendText` 和 `ctx.delay`；附带 `jsconfig.json` 与 SDK 引用，可在编辑器中检查参数和返回值。

[call-script.json](call-script.json) 演示左 Shift+Home、等待 30ms 和 50ms、Backspace。它删除光标到行首的内容；光标已在行首时可能合并上一行，Home 的含义也可能因编辑器而异。示例调用会真实发送输入，运行前应使用测试文档。

[close-tab-script.json](close-tab-script.json) 与 [new-tab-script.json](new-tab-script.json) 分别向原目标发送左 Ctrl+W、左 Ctrl+T，复用同一入口。它们不查询浏览器连接器；实际效果取决于原目标软件如何处理快捷键，不保证一定关闭或新建标签页。

输入沿用同包已有 `recipe` 动作的格式：

| 步骤 | 参数与行为 |
| --- | --- |
| `sequence` | `modifiers` 为明确左右侧的修饰键 VK，`keys` 为按顺序轻点的普通键 VK；整组保持修饰键。 |
| `tap` | `key` 为一个普通键 VK，不增加按下延迟。 |
| `text` | `text` 为字面 Unicode 文本，不经过剪贴板，不展开占位符。 |
| `delay` | `milliseconds` 为 0–60000 的整数，同步等待并受动作总时限约束。 |

`input.keyDownDelayMs` 可显式设置为 0–1000 的整数；省略时，`sequence` 查询本次原目标，WPS 系列与 mstsc 默认 10ms，其他进程为 0。显式 0 不会被默认值替换。单次按键等待合计最多 3000ms。

最多 32 步，步骤和默认目标查询合计不能超过 32 次宿主调用。含 `sequence` 的 32 步输入需显式设置 `keyDownDelayMs`；不会先执行 31 步再因调用预算失败。步骤形状、等待、时序和文本参数在执行前检查；VK 支持范围、修饰键组合、物理按住状态与原目标身份仍由每次桌面调用校验，后面的调用仍可能失败。总时限不是各步等待时限之和。

`guarded-recipe-script` 使用 [guarded-recipe.mjs](guarded-recipe.mjs)，适合只允许特定软件的步骤。除上述参数外，必须提供 `input.allowedProcesses`，例如 `["devenv", "code"]`；名单为 1–16 个不重复的进程基本名，只接受 ASCII 字母、数字、点、下划线和连字符，每项最多 64 字符，不含路径或 `.exe` 后缀，比较时忽略大小写。缺失或无效名单会在任何宿主调用前拒绝。

该入口始终查询一次本次原目标，即使显式指定 `keyDownDelayMs`。进程不在名单时返回 `target_process_not_allowed`，不发送输入或等待；目标查询计入调用预算，因此最多 31 步。名单与步骤在查询前固定，执行期间的输入对象改写不会改变配方。进程名称限制用于选择适用软件，后续每次桌面调用仍会校验原目标身份。

返回的 `inputSubmitted` 只表示输入调用已被系统接纳；只有等待步骤时为 `false`，不证明目标程序已完成业务操作。遇到目标变化、取消或其他错误时停止后续步骤，已发生的输入不会撤销，也不会自动重试。

同包的 `sequence`、`shortcuts`、`recipe` 仍使用 Node 运行时。两种入口都是可编辑示例，不要求业务脚本采用步骤数组；普通脚本也可直接按需要调用上述 API。


Node `recipe` 也会在首次桌面调用前检查整组参数、时序和 32 次调用预算，并快照步骤与嵌套按键数组。后续等待期间改写调用方输入不会改变已选步骤。只包含等待时，返回 `inputSubmitted:false`；参数预检不替代每次桌面 API 对原目标、物理按住状态、平台 VK 支持及取消的检查，也不会撤销已提交输入。

Node `recipe.mjs` 与纯准备 helper 也纳入本包 `jsconfig.json` 的 strict 检查；没有新增运行时依赖，安装时须保留同目录的 `.mjs` 文件。

`sequence` 与 `shortcuts` 两个 Node 入口也在首次异步查询前固定输入；`shortcuts` 会先检查全部 1–16 步的 keys 与 delayAfterMs，再开始派发。键值范围、数量和单次按下等待沿用公开桌面 API；只支持原有的 keyDownDelayMs，不新增嵌套 timing 输入。任一步运行时目标变化或取消仍会立即停止，已发生输入不回滚、不重试。


## 按原目标进程发送关闭快捷键

`source-close-script` 是固定的同步轻量入口，调用模板为 `source-close-script-call.json`，输入必须是空对象。它读取本次提交时捕获的原目标，要求进程名和进程ID明确、目标仍在前台且为应用窗口；身份未知或查询失败时不发送输入。

按进程文件名去除 `.exe` 后的基本名作大小写敏感匹配：`Acrobat` 发送左Ctrl+W（VK162、87），`chrome` 发送左Alt+F4（VK164、115），其余已确认进程也发送左Alt+F4。该入口不查询或关闭浏览器标签，不以其他PDF阅读器名称代替Acrobat，也不重新选择前台窗口。

每次只发送一个快捷键，不重试或补偿。返回 `inputSubmitted:true` 仅表示输入提交获确认，不证明窗口、标签或文档已关闭；程序可能显示保存提示。发送后回执未确认时以 `input_unconfirmed` / `unknown` 停止，不能据此重试。

## 折叠所有快捷键

`fold-all` 使用 Node，`fold-all-script` 使用同步轻量宿主；调用模板分别为 [fold-all-call.json](fold-all-call.json) 和 [fold-all-script-call.json](fold-all-script-call.json)，输入只接受空对象。

两个入口独立发送左 Ctrl+M，释放该段按键后等待50ms，再独立发送左 Ctrl+L；两段按下延迟均显式为0。它适用于配置了这组折叠快捷键的软件，不查询编辑器状态，也不保持 Ctrl 跨越两段。

每次输入沿用并复验本次原目标，不重新选择前台窗口。目标变化、取消或回执未确认会停止后续步骤；已提交输入不撤销、不重试。首段已提交后的失败以 `unknown` 描述整轮效果，错误消息保留已确认段数。返回 `inputSubmitted:true` 及 `acceptedSubmissions:2` 只确认两段输入提交，不保证目标程序已经折叠内容。

Node 与同步入口均通过 SDK JSDoc 和本包 `jsconfig.json` 检查；同目录的 `fold-all-support.mjs` 需随入口保留。

## 复制后编辑与类型检查

复制整个包目录时保留 `sdk.d.ts`、`sdk/` 和 `jsconfig.json`。Node 与同步 Script 入口使用包内声明，可脱离源码仓库获得补全与严格检查；无需依赖仓库的相对路径。声明固定于公开提交 `e51a45b8e312e4edc71ee4003039b0875842bb3e`，来源见 `sdk/README.md`。这些文件只提供编辑器类型，运行时仍由 OmniTrigger 注入 ctx。

两个折叠入口分别启用动作单实例：同一动作服务中，同一动作 ID 的前一调用未结束时，宿主拒绝再次启动，避免该入口的两段输入交错。Node 与 Script 是不同动作 ID；绑定时选用其中一个入口，不据此假定跨入口互斥。输入序列和 50ms 等待保持不变。

## 独立作者检查

在本工程目录中使用 Node.js 22 或以上版本执行：

```powershell
npm ci --ignore-scripts
npm run check
```

TypeScript 5.9.3 仅为开发依赖，版本与完整性由本包锁文件固定。检查使用本包 SDK，不需要仓库的 crates、tools 或 node_modules；strict、skipLibCheck:false、ES2020 与 types:[] 同时生效，覆盖清单全部入口及共享模块，不导入或执行作者动作。Node 上下文保持异步，Script 上下文保持同步，不向 Script 加入 DOM 或 Node 全局。

开发清单不设置 package.type，保留既有 .js 函数体与 .mjs 模块的解释。静态通过不代表输入已发送、原目标仍有效或宿主具备相应能力；旧函数体入口的加载和业务效果仍需运行时验证。
