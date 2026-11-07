# 检查已有 JavaScript 动作包

在仓库安装现有开发依赖后，可直接检查已有、附带本地 SDK 的 Node、轻量或混合动作包，不必启动应用或再创建一个工程：

```powershell
npm ci --prefix tools/script-sdk-check --ignore-scripts --no-audit --no-fund
node tools/script-sdk-check/check-project-types.mjs "D:\Actions\my-script"
```

同一命令读取明确工程中的 `action.json`、`jsconfig.json` 与同包 SDK（优先 `sdk/api.d.ts`，缺失时使用根目录 `api.d.ts`）。清单中的 Node 与轻量入口及其依赖都参与同一受限 TypeScript 工程；轻量 `.mjs` 另检查静态导入图和 `main` 具名导出，Node `.js/.mjs/.cjs` 复用现有 Node 默认导出分析。成功退出0，有类型错误、静态环境错误或工程错误退出1。它只做静态检查，不创建、修补或覆盖工程文件，不执行模块初始化器或入口，也不启动应用、宿主或编辑器。

原有 `compilerVersion/project/entries/diagnostics/checkedFiles/runtimeExecuted/filesModified` 字段保留；`diagnostics` 仍是 TypeScript 原数值诊断代码及工程内文件名、行列。新增 `environmentDiagnostics` 保存模块和入口诊断，`category` 分别是 `modules/entry`，代码为字符串。顶层 `passed` 要求类型和静态环境检查都没有错误，`staticEnvironmentPassed` 单独表示后者；警告不改变退出码。

Node 结果新增 `nodeEntryChecks`，按 Node 入口保留原检查器的诊断；轻量 `moduleChecks/entryChecks/skippedEntries` 的含义不变。Node 诊断也进入 `environmentDiagnostics`，保留原代码、文件和一基行列，并带有 `runtime:"node"`。`entries` 列出两类清单入口。Node 错误使 `staticEnvironmentPassed` 和 `passed` 为 false；未确认警告使 `staticAnalysisComplete` 为 false，类型诊断仍保留原 TypeScript 数字代码。纯轻量结果不增加 `nodeEntryChecks` 字段。

清单入口使用最近的 `jsconfig.json`：从入口所在目录逐级向上，最多到动作包根，逐个尝试固定文件名，不扫描目录。最近配置损坏或不可读取时直接拒绝，不回退父配置。声明 `files` 和编译选项中的相对路径以该配置所在目录为准，仍只能读取同一动作包内的普通文件及已定位标准库。

同一配置的入口共用原检查流程；不同配置分别建立 TypeScript 工程，保持各自环境。包根单配置的返回格式不变；使用子目录配置时新增 `projects`，列出配置、入口、结果和各自读取文件。聚合类型及环境诊断增加 `config` 标明所属配置；同一共享模块在不同环境中的诊断可以分别出现。工程文件数和总字节预算在整个动作包内累计，不因拆成多个配置而重置。

`moduleChecks/entryChecks` 按入口返回完整静态结果。轻量导入图需要显式 `.js`／`.mjs`，不补全扩展名，也不支持 Node 包导入；静态加载图最多64模块、合计1MiB。TypeScript 的类型解析仍受明确工程范围限制。若 TypeScript 将 `./value.js` 重定向到 `value.ts`，工具仍按宿主需要的明确 `.js` 路径补读包内静态依赖，沿用相同的元数据、UTF-8和容量防护，不枚举目录。

动态导入目标和再导出值无法静态确定时，返回警告并将 `staticAnalysisComplete` 设为 `false`。旧 `.js` 函数体入口继续接受类型检查，记录在 `skippedEntries`，不强加ESM导出要求；该字段也使静态分析完整性为 `false`。清单中的旧 `.js` 入口若出现静态 `import/export`，会另报可定位的 `legacy_entry_module_syntax`：该入口实际被宿主包装为函数体，不能按 ESM 执行；模块入口使用 `.mjs` 并具名导出 `main`。此限制只针对旧入口，不限制被 `.mjs` 导入的 `.js` 辅助模块。`main` 的函数声明不证明初始化之后仍可调用或返回合法结果。`runtimeCapabilitiesChecked/runtimeVerified` 始终为 `false`，不会探测实际宿主。更详细的静态边界见 [模块快照检查](MODULE_SNAPSHOT_CHECK.md)。

这适合使用记事本或需要命令行反馈的开发流程。保留入口的 `@type {ScriptMain}` 标注及工程SDK；检查使用工程自己的声明快照，不自动升级到仓库最新版。Node ESM 必须 default 导出可调用函数，检查也会等待类型层面的顶层 Promise，拒绝明确无法形成所需 JSON output 的函数、symbol 或 bigint 结果。CommonJS、动态类型和自定义序列化只报告未确认警告，不执行代码来猜测结果。

更新已有工程时，先保留原 SDK，再由作者显式选择与入口作用域相符的同版本快照：

| 工程入口 | SDK 选择 |
| --- | --- |
| `.mjs` 模块，或同时包含模块和旧入口 | 使用同版本新建模块工程的独立 `sdk/api.d.ts`；`ctx` 由 `main(ctx)` 向辅助函数显式传递。旧 `.js` 入口同样可以保留 `@type {ScriptMain}` 和形参。 |
| 旧 `.js` 仍依赖函数体内隐式 `ctx` | 使用同版本宿主包的旧入口 SDK。全局 `ctx` 只适用于该函数体协议，不能据此认定 ESM 模块中也存在全局上下文。 |

源声明若含 `reference path`，不能只复制包装文件而遗漏它引用的类型文件；工程本地 SDK 应保持自包含。安装或升级宿主不会修改作者的 SDK；替换声明也不会升级已经安装的宿主。更新后先运行本工具，再通过原有显式运行入口验证实际行为。

`jsconfig.json` 的编译选项会参与检查，始终启用 `allowJs/checkJs/noEmit`。纯轻量与混合包保持不自动加载环境类型；纯 Node 包默认也不发现环境类型，仅按作者明确的 `compilerOptions.types` 请求包内依赖。检查时固定 `moduleDetection:"force"`，使多个旧 `.js` 入口的 `main` 和局部变量分别属于各自文件，符合每次调用独立函数体的运行方式；这只影响类型作用域，不改写源码、不要求旧入口导出，也不改变运行协议。编辑器需要同样的隔离时，可在工程自己的 `jsconfig.json` 中设置该选项。当前不展开 `extends`；除清单入口和按需依赖外，也加入 `files` 逐个列明的包内 `.d.ts`，使混合 Node/同步 SDK 的显式环境声明与编辑器一致。`files` 不能隐藏清单入口，也不把清单之外的 Node 源码扩为动作入口；不展开 `include` 或目录通配。声明路径必须是相对该配置目录、解析后仍位于动作包内的普通文件，沿用元数据、UTF-8及容量防护；越界、绝对或通配路径返回定位到 `jsconfig.json` 的 `project_config_unsupported`，非字符串数组或超过256项为 `project_config_invalid`。缺失的显式声明保留 TypeScript 缺文件诊断。缺少SDK或配置分别报告 `project_sdk_missing/project_config_missing`，配置不支持或损坏为 `project_config_unsupported/project_config_invalid`。`jsconfig.json` 的 JSON 语法错误及 `compilerOptions` 错误值/未知选项另附 `error.location`，包含工程内文件名 `file` 和从 1 开始的 `line/column`；函数接口抛出的错误也保留相同 `location`，便于命令行定位，使用既有 `project_config_invalid` 错误码。

只使用 SDK 的纯 Node 包可使用 `types:[]`，Node API 的 Promise 仍会严格参与检查。使用 `process`、`node:fs` 等 Node 环境 API 的纯 Node 包，需要自己的开发依赖和 `types:["node"]`；工具不会安装或伪造声明，缺少类型时保留 TypeScript 的 2688、2307 等实际错误。显式类型依赖也必须位于当前包内，受同样的元数据、UTF-8、单文件和工程总量限制；不会到包外寻找全局 `@types/node`。这些限制可能使大型类型依赖报工程容量错误，应使用该工程原有的 TypeScript 检查流程。

同一配置同时包含 Node 和 Script 入口时，非空 `types` 仍返回 `project_mixed_types_unsupported`。需要 Node 内置模块的混合包可将入口放到不同目录，分别提供就近配置：Node 明确加载本包 `@types/node` 和 Node SDK，Script 使用 `types:[]` 与同步 SDK；共用模块可通过包内相对导入同时参与两边检查。示例见 [mixed-projects](../../examples/actions/mixed-projects/README.md)。共享模块自己的配置用于直接编辑和独立类型检查，统一命令只发现清单入口的配置，不枚举包里的所有配置。本命令不把 Node 的模块能力限制为轻量宿主的相对导入协议，也不声称检查了 Node 的实际模块加载或目标运行时。

内容读取限于该工程和已定位的TypeScript标准库。云盘命名目录、符号链接、硬链接及Windows云端占位属性会拒绝；不会扫描用户目录寻找SDK。单个源码最多128KiB、声明最多512KiB，工程读取合计最多8MiB、256个文件；超限为 `project_file_limit/project_total_limit`。文本要求UTF-8，错误为 `project_utf8_invalid`，不替换坏字节后继续检查。若错误发生在工程内文件，`error.location.file` 标明入口或依赖的相对路径；无法解码时不提供或猜测 `line/column`。标准库只读取已安装TypeScript中的明确文件。

该命令通过只表示当前声明和已覆盖的静态加载约束没有错误；应同时查看 `staticAnalysisComplete` 和警告。它不替代动作清单引用检查、实际模块加载、宿主能力或目标软件验收。查看实际调用行为仍使用项目现有的显式运行入口。测试全部使用合成目录，其中模块故意抛出初始化异常以证明检查没有执行代码：

```powershell
node --test tools/script-sdk-check/check-project-types.test.mjs
```

也可运行 `npm run test:project-types --prefix tools/script-sdk-check`。CI在安装现有SDK开发依赖后执行同一组合成测试，不启动应用或实际脚本宿主。
