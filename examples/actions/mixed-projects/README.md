# 一个动作包中的两类 JavaScript 工程

同一个 `action.json` 指向 `node/main.mjs` 与 `script/main.mjs`。两个入口共用 `shared/text.mjs`，分别使用 Node 异步 SDK 和轻量同步 SDK。Node 示例还使用真实的 `node:path` 与 `process` 类型。

在此动作包目录安装固定开发依赖并检查：

```powershell
npm ci --ignore-scripts
npm run check
```

依赖固定 TypeScript 5.9.3、`@types/node 22.20.5`；版本和完整性由本包 `package-lock.json` 记录。检查命令只运行三个配置的 TypeScript，无编译输出，不执行作者入口。实际动作运行仍由应用相应运行时负责。

在 VS Code 中打开整个动作包文件夹。内置 JS/TS 语言功能按入口就近发现配置：

| 目录 | 类型环境 | SDK |
| --- | --- | --- |
| `node/` | ES2022 与显式 Node 类型 | `sdk/api.d.ts`，API 返回 Promise |
| `script/` | ES2022，`types:[]` | `sdk/script-types.d.ts`，API 同步返回 |
| `shared/` | ES2022，`types:[]` | 纯 JS 模块，不依赖动作上下文 |

配置中的 `files` 相对当前配置目录，分别列明当前入口和所需声明。不要把两份 SDK 合并进一个配置，也不需要在包根添加汇总 `jsconfig.json`。Node 的 JSDoc 引用 `ActionContext`，Script 使用 `ScriptMain`；两边都能跳转到同一份共享函数定义。

共享源码会分别按导入方的环境参与检查；它自身的配置用于直接编辑和独立检查。共享函数使用 `unknown` 参数和标准 JS 返回值，避免依赖 `process` 等环境全局；若共享代码只在 Node 环境下成立，Script 工程会指出该文件中的错误。

在仓库已准备 SDK 检查工具的开发依赖后，也可运行同一个结构化检查入口：

```powershell
node tools/script-sdk-check/check-project-types.mjs "examples/actions/mixed-projects"
```

该命令从每个清单入口向上查找最近配置，分别检查两种入口合同及其依赖。`projects` 列出各配置的结果，共享模块的诊断携带 `config` 区分所属环境；它不扫描所有配置，独立共享配置由本包 `npm run check` 检查。最近配置损坏时直接报错，不回退父配置。读取仍限制在动作包内，允许包内 `../shared` 与 `../sdk`，拒绝越出包外的文件。

随包 SDK 是同版本声明快照，整个包可复制到配置旁的 `actions/`。显式更新 SDK 仍使用仓库既有 `tools/script-project.mjs refresh --runtime node --project <动作包目录>`；更新整组已识别声明，保留入口、配置与共享模块。声明更新不安装开发依赖或升级实际运行时。
