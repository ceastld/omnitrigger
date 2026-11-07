# 依次运行两个程序

Node 动作按 `steps` 的顺序等待两个程序完成；前一步失败便停止。每步提供 `program`、可选 `args` 与 `cwd`，不使用隐式 shell。`call.json` 里的 Node 示例需要调用环境已经能找到该程序，使用前替换成自己的程序与参数。

复制整个目录到所选配置同级的 `actions/process-sequence`，保留 `sdk/` 和 `jsconfig.json`。打开该目录可获得真实 Node SDK 补全：`ctx.process.run/capture/start` 均返回 Promise，须按需要等待。声明只提供类型检查，不安装运行时或授予执行权限。

使用 TypeScript 5.9 严格检查实际入口，不执行动作：

```powershell
tsc --project jsconfig.json --noEmit
```

SDK 使用包内相对路径，不需要原仓库目录、Node 类型包或全局 `ctx` 声明。
