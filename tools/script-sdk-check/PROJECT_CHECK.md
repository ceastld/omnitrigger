# 检查已有轻量脚本工程

在仓库安装现有开发依赖后，可直接检查已有、附带本地 `sdk/api.d.ts` 的轻量工程，不必启动应用或再创建一个工程：

```powershell
npm ci --prefix tools/script-sdk-check --ignore-scripts --no-audit --no-fund
node tools/script-sdk-check/check-project-types.mjs "D:\Actions\my-script"
```

同一命令读取明确工程中的 `action.json`、`jsconfig.json` 与 `sdk/api.d.ts`，检查清单里 `runtime:"script"` 的 `.js/.mjs` 入口及依赖，并对 `.mjs` 入口检查轻量宿主的静态导入图和 `main` 具名导出声明。成功退出0，有类型错误、静态环境错误或工程错误退出1。它只做静态检查，不创建、修补或覆盖工程文件，不执行模块初始化器或入口，也不启动应用、宿主或编辑器。

原有 `compilerVersion/project/entries/diagnostics/checkedFiles/runtimeExecuted/filesModified` 字段保留；`diagnostics` 仍是 TypeScript 原数值诊断代码及工程内文件名、行列。新增 `environmentDiagnostics` 保存模块和入口诊断，`category` 分别是 `modules/entry`，代码为字符串。顶层 `passed` 要求类型和静态环境检查都没有错误，`staticEnvironmentPassed` 单独表示后者；警告不改变退出码。

`moduleChecks/entryChecks` 按入口返回完整静态结果。轻量导入图需要显式 `.js`／`.mjs`，不补全扩展名，也不支持 Node 包导入；静态加载图最多64模块、合计1MiB。TypeScript 的类型解析仍受明确工程范围限制。若 TypeScript 将 `./value.js` 重定向到 `value.ts`，工具仍按宿主需要的明确 `.js` 路径补读包内静态依赖，沿用相同的元数据、UTF-8和容量防护，不枚举目录。

动态导入目标和再导出值无法静态确定时，返回警告并将 `staticAnalysisComplete` 设为 `false`。旧 `.js` 函数体入口继续接受类型检查，记录在 `skippedEntries`，不强加ESM导出要求；该字段也使静态分析完整性为 `false`。`main` 的函数声明不证明初始化之后仍可调用或返回合法结果。`runtimeCapabilitiesChecked/runtimeVerified` 始终为 `false`，不会探测实际宿主。更详细的静态边界见 [模块快照检查](MODULE_SNAPSHOT_CHECK.md)。

这适合使用记事本或需要命令行反馈的开发流程。保留入口的 `@type {ScriptMain}` 标注及工程SDK；检查使用工程自己的声明快照，不自动升级到仓库最新版。Node动作不在此工具范围内。

`jsconfig.json` 的编译选项会参与检查，始终启用 `allowJs/checkJs/noEmit` 并排除隐式环境类型。当前不展开 `extends`；只检查清单入口和按需依赖，不遍历 `include` 下未使用的其他文件。缺少SDK或配置分别报告 `project_sdk_missing/project_config_missing`，配置不支持或损坏为 `project_config_unsupported/project_config_invalid`。

内容读取限于该工程和已定位的TypeScript标准库。云盘命名目录、符号链接、硬链接及Windows云端占位属性会拒绝；不会扫描用户目录寻找SDK。单个源码最多128KiB、声明最多512KiB，工程读取合计最多8MiB、256个文件；超限为 `project_file_limit/project_total_limit`。文本要求UTF-8，错误为 `project_utf8_invalid`，不替换坏字节后继续检查。标准库只读取已安装TypeScript中的明确文件。

该命令通过只表示当前声明和已覆盖的静态加载约束没有错误；应同时查看 `staticAnalysisComplete` 和警告。它不替代动作清单引用检查、实际模块加载、宿主能力或目标软件验收。查看实际调用行为仍使用项目现有的显式运行入口。测试全部使用合成目录，其中模块故意抛出初始化异常以证明检查没有执行代码：

```powershell
node --test tools/script-sdk-check/check-project-types.test.mjs
```

也可运行 `npm run test:project-types --prefix tools/script-sdk-check`。CI在安装现有SDK开发依赖后执行同一组合成测试，不启动应用或实际脚本宿主。
