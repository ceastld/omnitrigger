# OmniTrigger

统一录制、注册与管理键盘和鼠标组合的 Rust 触发引擎。

录下一个操作，就能将其注册为触发器，例如 `CapsLock + W`、`鼠标侧键 + 滚轮上滚`。当前提供 Windows 桌面测试程序，触发后显示本地提示、发送快捷键、运行程序或脚本，也可呼出自定义动作轮盘；还可在动作库安装 RaiShot 插件，选择截图动作并设置触发。

项目处于 `0.1.0` 早期开发阶段，API 与配置格式可能调整。目前仅支持 Windows，不提供稳定版本或自动更新。

[源码仓库](https://github.com/ceastld/omnitrigger) · [贡献指南](CONTRIBUTING.md) · [安全说明](SECURITY.md) · [MIT 许可证](LICENSE)

## 下载 Windows 预览版

从 [Releases](https://github.com/ceastld/omnitrigger/releases) 下载 `OmniTrigger-<版本>-windows-x86_64.zip`，完整解压后运行 `omnitrigger.exe`。当前只提供 Windows x64 包，建议使用 Windows 11；Windows 10 尚未完成全面兼容验证。发布包包含项目及依赖许可，旁附 `SHA256SUMS.txt` 供核验下载完整性。

程序未经代码签名。运行需要 [Microsoft Visual C++ v14 Redistributable x64](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist)；如提示缺少 `VCRUNTIME140.dll`，请安装微软官方运行库。无需安装 Rust 或开发工具。预览版仍有下文说明的输入、焦点与权限限制。

## 当前状态

当前提供 Windows + GPUI 桌面测试程序，核心引擎可独立测试，主要能力包括：

- **统一触发描述**：键鼠组合、长按、快速拖拽与鼠标区域条件共用 `TriggerSpec`，录制、注册与运行时匹配采用同一套规则。
- **动作执行**：本地提示、发送快捷键、本地步骤、窗口操作、音量 / 亮度调节、外部程序及可选截图插件。
- **桌面界面**：触发器管理、23 个可编辑预设、浮动触发面板、多轮盘与按应用选择槽位。
- **配置与诊断**：本地自动保存、旧配置迁移备份、只读配置检查、输入诊断及监听重建。

详细交互、执行限制与界面说明见[当前功能](docs/FEATURES.md)，专题入口见[文档目录](docs/README.md)。尚未实现双击 / 连击、按设备区分、系统托盘、插件市场或自动更新。

## 使用 RaiShot 截图插件

1. 打开侧栏“动作库”，点击“安装 RaiShot 插件”。程序在后台下载并校验固定版本 `0.4.15` 的原发布包，安装完成后保存共享 CLI 路径。安装本身不启动截图，也不注册快捷键。
2. 在截图卡片点击“设置触发”，默认填入 `左 Ctrl + 左 Alt + A`。这是 OmniTrigger 适配提供的默认组合，可点击“重新录制”录制自己的组合，或“选择预设”改用长按、侧键等方式。
3. 点击“添加触发器”；编辑已有绑定时点击“保存动作”。“恢复默认”只修改当前草稿，仍需保存并通过冲突检查，不覆盖其他组合。
4. 使用组合触发截图，或从浮动面板点击动作。“测试截图动作”会真实启动截图；进程正常退出也可能是取消截图，不能据此认定已经生成图片。

已有 RaiShot 时，可填写 `raishot-cli.exe` 的完整路径后点击“保存并启用”，无需重新下载。动作库也提供刷新和停用；缺失程序、停用插件或不兼容动作会保留绑定并标记不可用，同时移出实际监听注册集。多个截图绑定与测试入口共享单并发，忙碌时拒绝新调用。

安装器访问 [RaiShot v0.4.15 发布包](https://github.com/ceastld/raishot/releases/tag/v0.4.15)，先尝试匿名 HTTPS。当前下载仓库需要访问权限；匿名下载不可用时，安装器尝试使用本机已安装、已登录且有仓库权限的 GitHub CLI（`gh`），不会自动安装或登录 `gh`。无权限时显示安装错误；也可使用自己已安装的程序路径。

下载包包含截图所需程序、DLL 和许可文件；安装器校验 ZIP 的 SHA-256 及包内文件后再启用，失败保留原设置，不自动重试。运行文件位于配置文件同级的 `plugins\raishot\runtime\0.4.15`。RaiShot 保留其 GPL-3.0-or-later 及随包第三方许可，详见[第三方说明](THIRD_PARTY_NOTICES.md)。当前不支持外部插件清单导入、插件市场或自动更新。

## 启动与验证

需要 Windows、Rust（MSVC 工具链）及 Visual Studio C++ Build Tools（含“使用 C++ 的桌面开发”和 Windows SDK）。仓库通过 `rust-toolchain.toml` 固定 Rust 1.90.0、rustfmt 与 Clippy；脚本需要 PowerShell 7。

```powershell
git clone https://github.com/ceastld/omnitrigger.git
cd omnitrigger
.\start.ps1                                      # 构建并打开界面
cargo run -p omnitrigger-app --locked             # 在终端运行
```

构建和检查：

```powershell
.\scripts\check.ps1                               # 与 Windows CI 共用，包含示例配置检查
cargo build -p omnitrigger-app --locked
cargo test --workspace --locked
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

真实 Hook 检查需要交互式 Windows 桌面，CI 不覆盖实际按键、输入焦点或权限差异：

```powershell
.\target\debug\omnitrigger.exe --smoke-test
cargo run -p omnitrigger-windows --locked --example hook_smoke
```

直接启动产物：`target\debug\omnitrigger.exe`。

上述路径为默认 Cargo 输出目录；`start.ps1` 和检查脚本共用 `scripts/build-app.ps1`，从本次 Cargo 构建回执定位 EXE，支持自定义 `CARGO_TARGET_DIR` 与 Windows 构建目标目录。非 Windows 环境可运行 `cargo test -p omnitrigger-core --locked` 验证核心；桌面程序仍仅面向 Windows。

也可通过 `--config <文件路径>` 使用独立配置，例如 `.\target\debug\omnitrigger.exe --config .\.local\demo\bindings.json`；后续编辑会保存到该文件，适合隔离试用。

使用 `--check-config` 可检查所选配置的格式、动作参数、组合冲突、同目录轮盘配置及轮盘引用，不安装 Hook 或执行动作：`.\target\debug\omnitrigger.exe --config .\examples\capslock-navigation.json --check-config`。

轮盘管理和显示入口：

```powershell
.\target\debug\omnitrigger.exe --wheel-editor
.\target\debug\omnitrigger.exe --wheel orbit-edit  # 显示指定轮盘，等待选择
```

两个入口均可配合 `--config` 使用；轮盘编辑与呼出组合设置见[自定义轮盘](docs/RADIAL_WHEELS.md)。

测试建议：先按住 `CapsLock` 再按 `W`，松开后确认提示浮层和列表中的命中次数；然后测试侧键加滚轮，最后录制一个自定义组合。单独按下并松开未被组合使用的前缀时，会回放原输入。

## 配置与隐私

启动时加 `--panel` 可直接显示浮动面板；可以同时指定 `--config` 使用独立配置。主窗口负责配置管理，关闭主窗口会退出应用；当前没有系统托盘。

绑定、提示文字、位置条件与动作参数保存在 `%LOCALAPPDATA%\OmniTrigger\bindings.json`；没有 `LOCALAPPDATA` 时使用系统临时目录下的 `OmniTrigger\bindings.json`。读取兼容版本 1–7，保存使用版本 7；首次保存旧配置前创建 `bindings.json.v<旧版本>.bak` 备份，备份失败不会覆盖原配置。回退旧程序时需恢复对应备份。

插件启停和共享程序路径保存在绑定文件同级的 `plugins.json`。使用 `--config` 时，插件设置和下载目录也位于所选配置文件的目录，便于隔离试用。旧程序动作保持原始参数，不自动转换成插件引用。

轮盘内容保存在绑定文件同级的 `wheels.json`，读取版本 1–2，保存版本 2；可选的 `position` 保存圈号、分区和半分区偏移，`app_overrides` 保存按进程文件名匹配的完整槽位替换。呼出组合在绑定文件中保存 `show_wheel` 动作与轮盘标识。每个槽位独立保存动作参数，不随复制来源的绑定更改。轮盘引用缺失时保留绑定并标记不可用；损坏或不支持的轮盘配置不会被保存操作覆盖，保存失败时保留原文件。配置形态见[自定义轮盘](docs/RADIAL_WHEELS.md)。

程序与参数逐项传递，不默认经过 Shell，不展开环境变量或引号，也不自动重试。外部程序按当前用户权限运行；仅启动自己信任的程序和脚本。进程正常退出不代表插件业务已完成，标准输出与错误输出当前不采集。暂停监听只阻止新触发，退出 OmniTrigger 不会终止已启动的外部程序。见[外部动作与脚本示例](docs/COMMAND_ACTIONS.md)。

当前应用不提供遥测、云同步或输入上传。插件安装会访问 GitHub 下载程序及运行依赖，不上传输入事件或截图；全局监听用于本机匹配与录制，触发记录和输入诊断仅保留在进程内。配置中的提示文字、诊断截图可能包含个人内容，报告问题前请自行脱敏。

## 与其他输入工具并用

Windows 的同类低级 Hook 按后安装在前的顺序处理。其他软件若在上游拦截 CapsLock、侧键或滚轮，OmniTrigger 就可能收不到原输入；上游补发的模拟输入仍按原策略过滤，避免动作自触发。仅凭漏触发不能认定具体来源。

- “输入诊断”页每秒更新键盘/鼠标回调数、最近事件时间、外部模拟输入数、当前按住的键，以及自身补发、切换窗口取消和补发失败次数；底部状态栏显示简要计数。
- 长按诊断显示实际判定耗时、配置阈值、判定后的通知与显示提交耗时，并分别累计移动、切窗与短按 / 其他取消。可据此区分超时显示与候选被取消；数据只在内存保存，显示提交耗时不等于显示器像素亮起的时刻。
- 偶发失效时，松开其他按键后点击“重建监听”：旧监听先按原释放协议收尾，再启动新输入线程、重新安装 Hook，并恢复绑定及暂停状态；清除旧线程残留的按键状态。保留消息、录制结果和用户配置。
- 程序不定时重装 Hook 抢链首；其他软件稍后再次重装时，仍可能改变输入顺序。
- 输入线程结束或重建失败时明确显示未监听，不再维持“正在监听”的假状态。

可控优先级检查：`cargo run -p omnitrigger-windows --locked --example hook_priority_smoke`。测试 Hook 仅拦截专用标记的 F24 模拟探针，验证上游吞事件及重建后顺序变化；不改变其他应用，也不将探针交给前台窗口。

## 核心设计

- **统一描述**：录制器生成 `TriggerSpec`，注册、冲突检查与运行时匹配共享这一份语义。
- **确定性状态机**：核心按 `InputEvent + 显式时间与上下文 → Engine::process_at / advance_time → Decision` 工作，鼠标位置和屏幕矩形由平台显式提供，输出透传、消费、触发通知及必要的补发计划。核心不读取系统时钟，不依赖 UI 或 Windows API；原 `process` 接口沿用最近显式时间。
- **平台适配**：Windows 适配层负责采集输入、执行消费决策和补发；系统 Hook 的同步路径只做有界匹配，不执行动作、网络请求或 Agent 推理。
- **程序化协调**：当前 Rust 库提供触发器注册与决策接口，冲突返回结构化诊断；桌面程序在 Hook 之外异步执行外部命令。工作流调用协议与自动分配尚未实现。
- **轻量界面**：GPUI 作为候选界面，负责录制、展示、编辑与必要确认，与核心引擎解耦。

插件采用“插件提供预设动作与默认快捷键，OmniTrigger 管理绑定”的方式。当前内置 RaiShot 截图清单，默认组合可自定义、恢复并经过统一冲突检查，执行仍在 Hook 之外。已实现范围与后续清单扩展见[插件动作与快速绑定设计](docs/PLUGIN_SYSTEM.md)；执行路径与扩展边界见[动作执行架构](docs/EXECUTION_ARCHITECTURE.md)。现有 Node 调用示例见[截图脚本](examples/raishot-capture.mjs)。

## 输入语义边界

“任意组合”是模型的扩展目标，实际能力受系统与硬件提供的事件约束。

已注册的前缀按下时暂存输入，组合命中后消费整次按压；遇到不匹配输入或单独松开时回放。延迟与补发会改变时序，不能承诺完全无损。前台窗口切换、暂停或替换注册列表时取消旧缓冲，避免向新窗口补发旧输入。

组合使用精确按下集合，区分左右修饰键；键盘长按默认只触发一次。滚轮以 Windows 标准的 120 增量为一格，高精度增量累积，正反方向分别统计。自身补发和其他软件注入不触发，也不参与录制。

本版支持“按住若干键 + 按下一键/滚轮方向”、目标键长按及鼠标快速拖拽，可分别附加一个鼠标位置条件，最多 64 条绑定、每条最多 8 个前缀。长按阈值为 50–10000ms，同一按压只触发一次；计时不依赖下一次输入。快速拖拽使用 `TriggerSpec` 的 `Activation::Drag`，可配置 4–500 个物理像素及 20–2000ms 的严格起步时窗；预设为 `18px / <180ms`，不绑定特定动作。

鼠标长按单独等待时，尚未到时位移超过 8px 会取消；与快拖共存时，快拖时窗内的 9px 小位移不会提前补发。快拖到期仍静止则继续等待长按；同次按压命中其中一项后不再触发另一项。额外按键、滚轮、切窗、暂停、录制和替换绑定会取消相应等待或拖拽会话，必要的释放收尾继续处理。右键短按或快拖失败后的补发会改变原始输入时序与拖动起点，不能承诺无损恢复；系统调度也会影响浮窗实际显示时刻。

重复组合、普通按下与同组合的长按或快拖争用、同一组合配置多个长按或多个快拖，以及终端键同时被其他组合用作前缀时会返回冲突；位置条件可证明互斥时允许分别绑定，相交或无法证明互斥时不暗设优先级。同组合的一个长按与一个快拖可以共存，但快拖时窗必须严格短于长按阈值。界面禁止不限位置的裸左/右键“按下即触发”；可明确选择区域条件、长按或快拖。暂不支持双击、多击、顺序连击或按设备区分；Windows 安全桌面和系统保留组合仍受系统限制。

普通权限运行；向权限更高的应用补发输入可能失败，此时暂停监听并显示错误。无需安装驱动。

## 工程结构

| 目录 | 职责 |
| --- | --- |
| `crates/omnitrigger-core` | 无外部依赖的模型、注册验证、状态机、录制器与区域计算；按职责分模块并保留统一公共 API。 |
| `crates/omnitrigger-windows` | Windows 输入采集、消费 / 补发、窗口与系统操作；隔离原生 API。 |
| `crates/omnitrigger-app` | GPUI 界面、持久化、动作分派、后台执行器与插件安装。 |
| `docs` / `examples` | 使用与架构文档、可通过 `--check-config` 验证的配置示例。 |
| `scripts` / `.github` | 本地检查入口、CI 和协作模板。 |

模块定位、事件流和后续拆分边界见[项目架构](docs/ARCHITECTURE.md)。

## 参与开发与许可证

问题与建议请使用仓库的 [Issues](https://github.com/ceastld/omnitrigger/issues)，贡献前阅读 [CONTRIBUTING.md](CONTRIBUTING.md)。Windows CI 检查格式、工作区测试、Clippy、应用构建和示例配置，Linux CI 独立检查核心测试与 Clippy；实际输入行为仍需桌面验证。

项目自身的代码、文档和资源采用 [MIT 许可证](LICENSE)，允许在保留版权及许可声明的前提下使用、修改、分发和商业使用。第三方依赖遵循各自许可证，参见 [第三方说明](THIRD_PARTY_NOTICES.md)。准备公开源码或分发程序时，参照 [发布检查](docs/RELEASING.md)。
