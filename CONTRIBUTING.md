# 参与贡献

欢迎提交问题、功能建议与改进。提交较大改动前，请先用 Issue 说明使用场景、预期行为和涉及的范围。

## 开发环境

当前应用面向 Windows，需要 Rust 的 MSVC 工具链、Visual Studio C++ Build Tools 及 PowerShell 7。仓库的 `rust-toolchain.toml` 固定 Rust 1.90.0 和所需检查组件。

```powershell
cargo build -p omnitrigger-app --locked
.\target\debug\omnitrigger.exe
```

## 代码边界

先阅读[项目架构](docs/ARCHITECTURE.md)定位模块，功能说明见[文档目录](docs/README.md)。

- `omnitrigger-core` 是纯 Rust 确定性状态机：输入标准化事件、时间与上下文，输出决策，不读取隐含全局状态。
- `omnitrigger-windows` 负责 Windows 输入采集、消费、补发和监听生命周期。Hook 同步路径只做有界状态更新与匹配。
- `omnitrigger-app` 负责 GPUI 界面、配置与提示消息。磁盘、网络和动作执行留在核心与 Hook 同步路径之外。
- 录制、注册、冲突检查和运行时匹配共用 `TriggerSpec`；不要静默改写用户录制的组合。

状态机改动应通过输入事件序列和预期决策验证，优先覆盖前缀回退、按键释放、重复事件、注入过滤、取消与恢复边界。新增行为后同步更新 README、相关文档与主页，只描述实际可用能力。

## 验证与提交

按改动执行以下检查；纯文档改动检查内容与差异即可。修改 `docs/` 或 `site/` 时，可在 `site` 目录运行 `npm ci` 与 `npm run build`，同步文档并检查站内链接，详见[从源码构建](docs/BUILDING.md#主页与文档站)。

```powershell
.\scripts\check.ps1
```

该脚本与 Windows CI 共用：依次检查格式、工作区测试、Clippy、应用构建，并用 `--check-config` 验证 `examples/*.json`，不会执行配置中的动作。需要单独运行时：

```powershell
cargo fmt --all --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build -p omnitrigger-app --locked
```

Linux / macOS 上可运行 `cargo test -p omnitrigger-core --locked` 和 `cargo clippy -p omnitrigger-core --all-targets --locked -- -D warnings`；Linux CI 以此检查核心独立性。平台与界面改动仍需 Windows 检查。

涉及 Windows Hook 生命周期时，在有交互桌面的 Windows 会话中额外执行：

```powershell
cargo build -p omnitrigger-app --locked
.\target\debug\omnitrigger.exe --smoke-test
# 或在完整检查后执行 Hook 检查：
.\scripts\check.ps1 -SmokeTest
```

该命令验证真实 Hook 的安装与卸载；自动化测试不能替代桌面中的组合录制、触发、释放和监听重建检查。请在 PR 中说明实际验证环境与结果。

用户说明与提交说明使用中文，提交格式为 `type(scope): 中文说明`，例如 `fix(core): 修复前缀释放时的回退`。PR 说明应写清问题、改动后的行为与验证结果。

不要提交本机配置、凭据、真实输入记录、构建产物或临时文件。应用配置中的提示消息也可能含有个人信息；复现问题时请使用最小化示例。

提交贡献即表示同意将该贡献按本项目的 [MIT 许可证](LICENSE) 提供。
