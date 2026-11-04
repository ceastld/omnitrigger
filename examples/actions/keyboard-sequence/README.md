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

返回的 `inputSubmitted` 只表示输入调用已被系统接纳；只有等待步骤时为 `false`，不证明目标程序已完成业务操作。遇到目标变化、取消或其他错误时停止后续步骤，已发生的输入不会撤销，也不会自动重试。

同包的 `sequence`、`shortcuts`、`recipe` 仍使用 Node 运行时。两种入口都是可编辑示例，不要求业务脚本采用步骤数组；普通脚本也可直接按需要调用上述 API。
