# 第三方依赖与资源

[MIT 许可证](LICENSE) 适用于本项目自身的代码、文档和资源。第三方依赖保留各自的版权与许可要求；项目的 MIT 声明不替代这些要求。

## Rust 依赖

精确版本以提交的 `Cargo.lock` 为准。主要直接依赖如下，包含构建工具：

| 依赖 | 用途 | 许可证声明 |
| --- | --- | --- |
| GPUI | Windows 桌面界面 | Apache-2.0 |
| windows | Windows API 绑定 | MIT OR Apache-2.0 |
| serde / serde_json | 本地配置序列化 | MIT OR Apache-2.0 |
| getrandom | 桌面桥接随机令牌与脚本工程稳定标识，复用锁定的 0.3.4 | MIT OR Apache-2.0 |
| unicode-segmentation | 文本编辑的字素边界 | MIT OR Apache-2.0 |
| raw-window-handle | 原生窗口句柄 | MIT OR Apache-2.0 OR Zlib |
| embed-resource | 构建时嵌入 Windows 图标资源 | MIT |
| omnitrigger-browser | 固定 Git 修订的浏览器 RPC 客户端 | MIT |
| tokio | 浏览器连接的后台异步运行时 | MIT |
| semver | 应用更新版本排序，复用锁定的 1.0.28 | MIT OR Apache-2.0 |
| sha2 | 脚本开发包及托管宿主映像完整性检查，复用锁定的 0.10.9 | MIT OR Apache-2.0 |

原生亮度提示通过现有 `windows` 绑定调用系统自带的 USER32、GDI 与 GDI+，没有增加第三方 Rust 包或捆绑系统 DLL。文字使用本机系统字体渲染，不分发字体文件。

浏览器动作通过 `omnitrigger-browser` 的 Rust 客户端直接复用本机持久连接，依赖固定于公开仓库修订 `a5adc50022120cc4e6b0596bb56e14ed0ec2d02e`，不随应用分发独立浏览器程序。该客户端与其传递依赖的许可由同一发行收集脚本保留；浏览器扩展和可选连接器的独立发行包另行保留其许可通知。

`OR` 表示可按上游条款选择其中一种许可。完整依赖图还包含传递依赖，不能仅凭此表打包可执行文件。

可查看当前 Windows 目标的依赖与包元数据：

```powershell
cargo tree --locked --target x86_64-pc-windows-msvc
cargo metadata --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc
```

初次准备开源时，核对了锁定依赖的许可证声明与包内的许可、版权文件。GPUI 0.2.2 使用 Apache-2.0，其许可文件包含 Zed Industries, Inc. 的版权声明；部分传递依赖另有 COPYRIGHT 文件。全平台依赖中还含 MPL-2.0 的 `cbindgen`（其他平台的构建依赖），扩展平台时需重新检查。

Windows 发行流程通过 `scripts/collect-licenses.ps1` 从锁定目标的应用可达依赖图收集许可全文、版权与 NOTICE；为保守起见同时保留构建依赖通知，并收集固定 Rust 工具链提供的标准库版权和许可。ZIP 中的 `licenses/dependencies.json` 记录包版本、原始许可声明、上游来源和随包文件 SHA-256，不包含构建机绝对路径。

部分上游 crate 声明了许可证但发布包未附带全文。受控补充文件的[来源说明](scripts/license-overrides/README.md)和[清单](scripts/license-overrides/manifest.json)按包版本固定来源、哈希和说明，随包保留原 Cargo / README 授权声明。优先使用该版本的上游源码文件；只有后续补充或标准许可正文时明确标注来源，不将其冒充发布包原文件，也不推断缺失的版权年份。这些第三方文件遵循各自许可证，项目 MIT 声明不替代它们。

新增未审核的许可声明或缺失许可文件会使打包失败；依赖变更后需重新审核和生成。该流程不能替代针对实际发行内容的审查。Apache-2.0 的发行要求见[官方许可第 4 节](https://www.apache.org/licenses/LICENSE-2.0)。

## 可选的 Node.js 运行环境

`tools/script-sdk-check` 仅用于开发与 CI 的工程生成和类型校验，锁定 TypeScript 5.9.3（Apache-2.0）。已核对 npm 包声明及随包 `LICENSE.txt`；上游见 [TypeScript 许可](https://github.com/microsoft/TypeScript/blob/v5.9.3/LICENSE.txt)。该编译器不内嵌到应用、脚本宿主或生成的 SDK 快照，动作运行不要求安装它；新建 Node 工程仅在作者显式安装开发依赖后使用它检查代码。版本、下载来源及完整性记录在该目录的 `package-lock.json`。

新建 Node 工程的开发清单固定 TypeScript 5.9.3（Apache-2.0）与 `@types/node 22.20.5`（MIT，DefinitelyTyped）；后者声明依赖 `undici-types ~6.21.0`（MIT，Node.js Undici）。已核对官方 npm 元数据、完整性与随包 LICENSE；来源为 [Node 声明](https://www.npmjs.com/package/@types/node/v/22.20.5)、[Undici 声明](https://www.npmjs.com/package/undici-types/v/6.21.0)。这些依赖仅由作者显式执行 `npm install --ignore-scripts` 安装到自己的工程；生成器不下载或执行 npm，SDK 不包含它们的源码，也不引入 npm 运行依赖。实际依赖版本和完整性由工程自己的 `package-lock.json` 记录。

本地 JavaScript 动作包使用用户安装的 Node.js。当前应用不下载或分发 Node 二进制，不引入 npm 运行依赖；内嵌的 JS 宿主与仓库示例由本项目实现，按项目 MIT 许可证提供。`omnitrigger-actions` 复用工作区已锁定的 `serde` 和 `serde_json`，没有新增第三方 Rust 包。

动作包作者自行引入的代码、模块、程序或素材遵循各自许可证，不能因包由 OmniTrigger 加载而视为 MIT。将来如将 Node 或其他运行时并入发行包，应按实际分发内容重新核对并保留完整许可和版权通知。

## 独立的 JS 引擎体积实验

`tools/js-engine-probe` 是独立 Cargo workspace，使用单独的锁文件，不进入桌面应用依赖图或正式发行包。动作服务的 `script` 运行时可启动本机构建的独立宿主，适配层本身不链接或打包引擎。宿主使用 Boa 0.21.1（`Unlicense OR MIT`，选择 MIT）及 rquickjs 0.14.0（MIT）；后者静态编译随 crate 提供的 QuickJS-NG C 引擎（MIT）。宿主、适配层与测试由本项目编写。

已核对锁文件对应 Cargo 元数据和 rquickjs / QuickJS-NG 随包许可。Boa 系列发布 crate 未附带许可全文，上游授权见 [Boa 0.21.1 MIT 许可](https://github.com/boa-dev/boa/blob/v0.21.1/LICENSE-MIT)。实验传递依赖含 MIT、Apache-2.0、Unicode-3.0、Unlicense 和 Zlib 等声明；双重授权可选择相应分支，Unicode 数据仍须保留其自身许可。完整版本与依赖关系以实验的 `Cargo.lock` 和 `cargo metadata --locked` 为准。

包内 ESM 加载启用 rquickjs 0.14.0 的 `loader` 特性，不启用 `dyn-load`。其 relative-path 2.0.3 依赖已在独立锁文件中；已核对发布 crate 的元数据与随包 LICENSE-MIT / LICENSE-APACHE（`MIT OR Apache-2.0`，选择 MIT）。开发包的完整依赖通知须继续包含这项可达依赖及其原始版权与许可。

测量脚本生成的 ZIP 只含 EXE，用于比较压缩体积，不是可发布安装包。CI 仅上传测量 JSON，不分发实验二进制。若正式分发宿主，须为其独立依赖图、标准库和 QuickJS-NG 收集完整许可及版权通知，补齐 Boa 发布 crate 缺失的上游许可文件，并记录来源和哈希；`small` 还链接 MSVC 静态 CRT，须一并核对其再分发要求。现有桌面应用的许可收集范围不能代替这一流程。

`tools/js-engine-probe/package-runtime.ps1` 另行生成仅供本机验证的 QuickJS 开发包；它按独立工程、入口包和 feature 调用许可收集器，保留可达正常及构建依赖、Rust 标准库、QuickJS-NG 和包装层通知，并记录文件摘要。rquickjs-core / rquickjs-sys 0.14.0 的包装层 MIT 全文按发布包 VCS 修订补充，来源与摘要见受控清单；sys 包原有的 QuickJS 通知同时保留。开发包记录 MSVC 静态 CRT 构建环境，公开二进制分发的再分发复核仍未完成；该流程不发布二进制或创建 Release。

脚本原型的桌面 API 复用本项目 MIT 的 `omnitrigger-windows`，引入的 Windows 绑定及其辅助 crate 为已在主 workspace 使用的 `windows` 0.61.3 依赖系列（MIT OR Apache-2.0）。精确版本记录在原型独立锁文件中；宿主桥接、JS SDK、类型声明和示例由本项目编写，按 MIT 提供。原型的 `small` 版仍按上文独立分发规则核对许可。

## 图标与字体

程序图标的 SVG 源文件与 ICO 位于 `crates/omnitrigger-app/assets/`，界面小图标以内联 SVG 保存在 `src/assets.rs`。这些文件随项目源码提供。引入外部素材时，应同时记录来源并保留其许可声明。

界面请求使用系统安装的 Microsoft YaHei UI、Cascadia Mono 等字体，仓库没有附带字体文件。系统字体不因本项目使用 MIT 而获得再分发许可。

## 主页与文档站

`site/` 使用 VitePress 构建主页与文档站，精确版本以 `site/package-lock.json` 为准。这些 npm 依赖只用于网站，不进入 Windows 发行包。

构建时，VitePress 运行时、Vue、本地搜索以及文档流程图使用的 mermaid 等软件包会被打包进站点的浏览器脚本。[`site/scripts/third-party-licenses.mjs`](site/scripts/third-party-licenses.mjs) 按实际打包的模块收集许可正文与版权声明，生成随站点发布的 `third-party-licenses.txt`。其中多数为 MIT 或 ISC，部分 `d3` 旧版本为 BSD-3-Clause，`dompurify` 为 MPL-2.0 OR Apache-2.0；个别发布包未附带许可正文时，清单注明其 `package.json` 中的许可声明。

站点使用访问者系统中的字体，不分发字体文件，也不加载外部字体、统计或广告脚本。界面截图与社交预览图来自本项目程序和图标，按 MIT 提供；预览图中的文字由系统字体渲染为图像，不包含字体文件。

## 可选下载的 RaiShot 运行程序

RaiShot 截图适配描述和 OmniTrigger 安装器属于本项目代码；截图能力由用户选择安装后独立下载的 [RaiShot v0.4.15 发布包](https://github.com/ceastld/raishot/releases/tag/v0.4.15) 提供。该外部程序使用 GPL-3.0-or-later，不能将其二进制、运行库或素材视为本项目 MIT 资源。

安装器下载原始 `RaiShot.0.4.15.0.zip`，校验固定 SHA-256、包内清单和文件哈希，并保留发布包中的 `LICENSE`、`THIRD_PARTY_NOTICES.md`、运行库许可及第三方说明。该包包含三个 RaiShot 程序、VC 运行库和 ONNX Runtime DLL；其具体版权、许可及通知以随包文件为准。截图动作不额外下载 OCR 模型。

本仓库没有复制这些外部二进制，也没有因此新增 Cargo 依赖。插件依赖当前按用户操作下载到本机配置目录；如将来把 RaiShot 或其运行库并入 OmniTrigger 发行包，需按相应上游许可另外核对分发要求并保留完整通知。

`examples/actions/raishot` 为独立的三模式消费者示例，需要显式配置本机 PowerShell 解释器；它在需要准备依赖时下载 [RaiShot v0.4.18](https://github.com/ceastld/raishot/releases/tag/v0.4.18)。该版本自身代码采用 [MIT](https://github.com/ceastld/raishot/blob/ab0cf17cd707a0e757567dc000ea9e45c0d51d4f/LICENSE)，运行库和其他组件仍受各自随包许可约束。示例保留原始 ZIP、全部程序/运行库及许可通知，并按固定发布包 SHA-256、清单和文件哈希校验；没有把外部二进制复制进本仓库，也不改变上述内置安装器的版本或行为。

## 可选 N10 Rust 适配示例

`examples/actions/n10-rust` 的清单、JavaScript 适配代码、文档和禁用配置草稿由本项目编写，按 MIT 提供，复用已有 script SDK，不新增第三方库或 Cargo/npm 依赖。

该包仅调用明确指定的本地 eaclip-cli 与 Rust 宿主，没有复制或分发外部原生二进制、运行库、源码及素材。外部程序由独立本地安装提供，其许可证、版权和依赖通知须随相应制品保留，不能由本项目 MIT 声明替代。脚本宿主与引擎仍适用本文件已有的独立许可及开发分发说明。
