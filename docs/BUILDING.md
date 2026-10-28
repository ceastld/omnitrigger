# 从源码构建

本页说明从源码构建、运行和检查 OmniTrigger。只想试用时，下载 Windows 预览版即可，见[快速开始](GETTING_STARTED.md)。

## 环境要求

需要 Windows、Rust（MSVC 工具链）及 Visual Studio C++ Build Tools（含“使用 C++ 的桌面开发”和 Windows SDK）。仓库通过 `rust-toolchain.toml` 固定 Rust 1.90.0、rustfmt 与 Clippy；脚本需要 PowerShell 7。

```powershell
git clone https://github.com/ceastld/omnitrigger.git
cd omnitrigger
.\start.ps1                                      # 构建并打开界面
cargo run -p omnitrigger-app --locked             # 在终端运行
```

直接启动产物：`target\debug\omnitrigger.exe`。

上述路径为默认 Cargo 输出目录；`start.ps1` 和检查脚本共用 `scripts/build-app.ps1`，从本次 Cargo 构建回执定位 EXE，支持自定义 `CARGO_TARGET_DIR` 与 Windows 构建目标目录。非 Windows 环境可运行 `cargo test -p omnitrigger-core --locked` 验证核心；桌面程序仍仅面向 Windows。

## 构建和检查

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

## 隔离试用与配置检查

通过 `--config <文件路径>` 使用独立配置，例如 `.\target\debug\omnitrigger.exe --config .\.local\demo\bindings.json`；后续编辑会保存到该文件，适合隔离试用。

使用 `--check-config` 可检查所选配置的格式、动作参数、组合冲突、同目录轮盘配置及轮盘引用，不安装 Hook 或执行动作：

```powershell
.\target\debug\omnitrigger.exe --config .\examples\capslock-navigation.json --check-config
```

轮盘管理和显示入口：

```powershell
.\target\debug\omnitrigger.exe --wheel-editor
.\target\debug\omnitrigger.exe --wheel orbit-edit  # 显示指定轮盘，等待选择
```

两个入口均可配合 `--config` 使用；轮盘编辑与呼出组合设置见[自定义轮盘](RADIAL_WHEELS.md)。

## 主页与文档站

主页与文档站位于 `site/`，使用 VitePress 构建，需要 Node.js 20 或更高版本。文档页由 `site/scripts/sync-docs.mjs` 从仓库 Markdown 同步生成，仓库文档仍是唯一来源；`docs/` 下新增文档需要在该脚本中登记侧栏位置。

```powershell
cd site
npm ci
npm run dev       # 本地预览，修改仓库文档后自动同步
npm run build     # 生成 site/.vitepress/dist，并检查文档链接
```

推送到 `main` 后，`Pages` 工作流构建并发布到 GitHub Pages。代码边界、验证要求与提交规范见[贡献指南](../CONTRIBUTING.md)。
