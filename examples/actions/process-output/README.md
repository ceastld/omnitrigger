# 运行自有程序后显示输出

明确指定本地绝对 EXE、参数和工作目录，等待完成后仅通知标准输出。示例路径是待替换的自有测试夹具占位路径，不是已安装程序。没有隐式 shell、PATH 搜索、脚本文件读取或外部日志；不创建调度。

输入接受 `program`、`cwd`（都必填）、`args`（最多128个字符串，默认空）、`allowNonZero`（默认false）、`trimOutput`（默认true）。两个 capture 入口另外接受 `timeoutMs`（1–60000，默认10000）。参数全部先验；原始输出仅供脚本内部，不写入动作终态。

三个入口有明确差异：

- `notify-script` 使用新增的 Windows10+ `process.capture`：stdout/stderr 合计64KiB，严格UTF-8；私有Job在创建时关联，宿主退出时按Job范围收尾。需更新后的轻量宿主。
- `notify-capture` 使用 Node 异步 `process.capture`：与轻量入口复用相同的严格采集参数和结果处理，必须等待采集与通知完成；需要包含此能力的新应用和 Node 宿主。缺失明确失败，不回退 `run`。
- `notify` 使用现有 Node `process.run`：沿用动作总运行时限，不接受单独 timeoutMs。现有宿主使用有损UTF-8解码，配方无法恢复原始字节判断坏编码；其取消只针对受管直接子进程，不能声称具备新capture的Job/整组收尾。没有偷偷把两种契约说成相同。

三个入口都显式要求cwd，避免各宿主默认工作目录不同。脚本用明确的解释器EXE及参数，例如调用方自行选择UTF-8输出模式；配方不猜程序位置、不启动原有runner或读取旧脚本。

默认非零退出失败且不通知；明确 allowNonZero=true 时才允许非零返回，并以warning级别通知stdout。stderr计入总量但不显示。trimOutput使用JS trim处理显示副本，保存的统计仍为原长度；空白stdout显示固定完成提示。通知最多1024 UTF-16且符合现有纯文本控制字符约束，不偷偷截断；无法显示时明确报错，程序可能已经产生效果。

运行或通知失败均无自动重试、无备用程序。后续通知取消/不可用不撤销程序已完成的副作用，整体effectState保留unknown。成功终态只含退出码、两路长度和notified；错误消息固定，无正文、程序路径、参数或cwd。

本配方不读取或重新捕获前台，不发送键鼠，也不使用剪贴板。通知按既有接口不激活窗口；原提交目标上下文保留，但此业务不向目标注入输入。真实程序、Job、取消和通知效果仍须在受控夹具中验收。

```powershell
node --test examples/actions/process-output/recipes.test.mjs
```

测试只用RecordingHost，不启动任何进程或真实宿主。实际运行调用示例将执行调用方明确填入的EXE。

新增 `call-capture.json` 指向 Node 的 notify-capture，显式给出 timeoutMs；旧 notify 仍不接受该字段，旧 notify-script 保持同步入口。三个动作分别 single_instance，不互锁。采集预算、错误与收尾范围见[动作包 API](https://github.com/ceastld/omnitrigger/blob/main/docs/ACTION_PACKAGES.md)，预算不是硬 OS 调用超时，外层动作期限可能更短。

复制整个目录到所选配置同级的 `actions/process-output`，保留 `sdk/`、`types.d.ts` 和 `jsconfig.json`。打开目录后，Node 两入口使用异步 SDK，同步入口使用 `ScriptMain`；共享 workflow 的类型重载保留两者的结果差异。同步 `process.run` 原本返回退出状态，不能当成 `capture` 的 stdout 结果。声明不安装宿主、改变调用方式或授予运行权限。

使用 TypeScript 5.9 严格检查实际三个入口和共享 helper，不执行动作：

```powershell
tsc --project jsconfig.json --noEmit
```

这些声明只引用包内文件，不依赖原仓库目录、Node 类型包或全局 `ctx`。上面的 RecordingHost 回归命令在源码仓库内运行；运行时入口的类型检查不包含测试脚本。
