# N10 剪贴板适配

### 日常剪贴板

在动作库选择「切换独立剪贴板」，可以把它绑定到 CapsLock + Q。第一次触发时插件按需启动 EaClip，再次触发可收起窗口。窗口沿用 EaClip 的历史、收藏和采集设置，选择记录后可贴回触发热键时的窗口。退出 OmniTrigger 后，已启动的 EaClip 仍按自己的设置运行。

这个入口使用 Node.js 和支持目标窗口校验的 EaClip 客户端及宿主。动作输入包含三个本机绝对路径：`cliExecutable`、`hostExecutable` 和 `dataRoot`。填写正在使用的 EaClip 数据目录即可继续访问其中的记录。程序与客户端可由 EaClip 源码中的 `host/Build-Standalone.ps1 -IncludeCli` 配套构建。

### 已运行宿主的脚本入口

原有的脚本入口通过已有的 `script` 运行时调用本地 Rust CLI，再由对应宿主解释命令。它不包含原生程序、不安装或启动宿主、不注册触发器，也不会自行选择、迁移或改写数据根配置。

脚本动作“剪贴板 n10”，动作标识为 `32c12786-9bb8-4b0c-8d55-7e6a4c8a5d10`，单实例由现有 ActionService 执行。入口直接使用 `n10-rust-bound.mjs` 导出的 `main`；包加载只定义函数。模块中的独立准备函数没有登记成动作，业务调用不会调用它。

需要已有轻量脚本宿主，以及支持显式数据根与原目标快照协议的 `eaclip-cli.exe` 和 Rust 宿主。原生依赖须来自明确、已校验的本地安装；它们与动作包分别管理。仅有一个旧 CLI 文件不代表支持本协议。

轻量脚本宿主按所选配置目录安装；源码示例目录不会继承其他配置的安装。开发检查可用已有 `OMNITRIGGER_SCRIPT_HOST` 环境变量显式指定本地轻量宿主，未安装或未指定时引用检查会返回 `runtime_unavailable`。安装方式见[动作包说明](../../../docs/ACTION_PACKAGES.md)。

`call.json` 是未配置模板，四个必填项故意为 null。实际调用的 `input` 必须且只能包含以下六项，不会从 PATH、环境变量、个人配置或包目录猜测程序与数据位置：

| 字段 | 要求 |
| --- | --- |
| cliExecutable | 明确的普通本地绝对路径，文件名为 eaclip-cli.exe |
| hostExecutable | 与该 CLI 配套宿主的普通本地绝对路径 |
| dataRoot | 本次调用预期使用的普通本地绝对数据根 |
| profile | 明确选择 product 或 script-test；不自动切换 |
| timeoutMs | 1–59000 的整数，仍受原目标期限和宿主调用总时限限制 |
| actionParameter | `{ "present": true, "value": null }` 或 value 为字符串 |

参数明确为 null 或空串时发送 `clip toggle`；非空字符串原样发送，不额外翻译。缺少参数、present 不是 true、非字符串或超出入口限制会在调用前拒绝，不推断成默认值。输入路径拒绝父级遍历、云盘、UNC 和链接相关不安全用法；本地物理文件的最终检查仍由已有进程 API 与 CLI 负责。

product 使用显式预期根；script-test 只连接显式 Debug 隔离测试宿主的私有端点，不能启用旧 test-instance 的 RPC，也不能回退产品实例。测试宿主仅允许约定的窗口命令，其他命令由宿主拒绝。本包不提供宿主准备、依赖下载或迁移入口。

调用时从已有 `ctx.desktop.targetContext()` 取本次提交捕获的原目标，句柄和 FILETIME 保持字符串精度，不重新挑选当前前台窗口。业务入口始终 `--no-start`、最多一次 capture，不自动重试。原目标不可用、已过期、依赖缺失或配置不匹配时通过既有错误通道返回；数据根和目标在原生宿主入口继续复验。

正常返回的 `status=accepted`、`commandEffectState=accepted_only` 只表示原业务入口接纳，`uiRendered=false`；它不表示像素渲染、粘贴或完整业务完成。unknown/partial 抛结构化错误，effectState 为 unknown，并在既有 code 和 N10_DETAIL 消息中保留 submitted 的 true/false/unknown 证据；不自动重试。

现有动作状态和“查看结果”显示本次脚本调用状态；脚本返回结果不表示外部窗口已经显示或文本已经发送。

源码目录的 `examples/n10-rust.json` 只有一条禁用草稿绑定，使用 CapsLock+R 作为占位。它不会自动启用或合并到当前配置。启用前先填入明确依赖和数据根、选择 profile，再自行录制触发方式并检查冲突。

将整个包目录放到指定配置同级 `actions` 下即可使用现有目录发现和重新加载。当前没有通用插件导入 UI、原生依赖自动安装、N10 专属插件就绪卡片或逐字段业务参数表单；可使用已有 JSON 输入。目录发现/引用检查只证明清单与入口可解析，不校验这四个必填配置或宿主是否可用；未配置模板的真实调用会明确拒绝。

本包代码采用随附 MIT 许可证；原生依赖和脚本引擎的独立许可见 THIRD_PARTY_NOTICES.md。
