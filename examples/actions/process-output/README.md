# 运行自有程序后显示输出

明确指定本地绝对 EXE、参数和工作目录，等待完成后仅通知标准输出。示例路径是待替换的自有测试夹具占位路径，不是已安装程序。没有隐式 shell、PATH 搜索、脚本文件读取或外部日志；不创建调度。

输入接受 `program`、`cwd`（都必填）、`args`（最多128个字符串，默认空）、`allowNonZero`（默认false）、`trimOutput`（默认true）。轻量入口另外接受 `timeoutMs`（1–60000，默认10000）。参数全部先验；原始输出仅供脚本内部，不写入动作终态。

两入口有明确差异：

- `notify-script` 使用新增的 Windows10+ `process.capture`：stdout/stderr 合计64KiB，严格UTF-8；私有Job在创建时关联，宿主退出时按Job范围收尾。需更新后的轻量宿主。
- `notify` 使用现有 Node `process.run`：沿用动作总运行时限，不接受单独 timeoutMs。现有宿主使用有损UTF-8解码，配方无法恢复原始字节判断坏编码；其取消只针对受管直接子进程，不能声称具备新capture的Job/整组收尾。没有偷偷把两种契约说成相同。

两个入口都显式要求cwd，避免各宿主默认工作目录不同。脚本用明确的解释器EXE及参数，例如调用方自行选择UTF-8输出模式；配方不猜程序位置、不启动原有runner或读取旧脚本。

默认非零退出失败且不通知；明确 allowNonZero=true 时才允许非零返回，并以warning级别通知stdout。stderr计入总量但不显示。trimOutput使用JS trim处理显示副本，保存的统计仍为原长度；空白stdout显示固定完成提示。通知最多1024 UTF-16且符合现有纯文本控制字符约束，不偷偷截断；无法显示时明确报错，程序可能已经产生效果。

运行或通知失败均无自动重试、无备用程序。后续通知取消/不可用不撤销程序已完成的副作用，整体effectState保留unknown。成功终态只含退出码、两路长度和notified；错误消息固定，无正文、程序路径、参数或cwd。

本配方不读取或重新捕获前台，不发送键鼠，也不使用剪贴板。通知按既有接口不激活窗口；原提交目标上下文保留，但此业务不向目标注入输入。真实程序、Job、取消和通知效果仍须在受控夹具中验收。

```powershell
node --test examples/actions/process-output/recipes.test.mjs
```

测试只用RecordingHost，不启动任何进程或真实宿主。实际运行调用示例将执行调用方明确填入的EXE。
