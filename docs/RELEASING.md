# 开源与发布检查

当前准备范围是公开源码。仓库中的 Windows CI 负责格式、测试、Clippy 与构建；不自动发布版本、上传程序或更改仓库可见性。三个 crate 暂设 `publish = false`，没有将其作为 crates.io 包发布。

## 公开源码前

- 核对 README 的功能、支持平台与命令，保留早期开发状态和实际输入限制。
- 检查待公开的文件及 Git 历史，不包含凭据、个人配置、真实输入记录、本机日志或构建产物；不要用重写历史代替必要的凭据撤销。
- 确认新增代码和素材具备授权，保留 [LICENSE](../LICENSE) 与[第三方说明](../THIRD_PARTY_NOTICES.md)。
- 执行下列检查，查看 `git diff --check` 与最终差异；实际 Hook 行为在交互式 Windows 桌面中验证。
- 由维护者确认公开范围后提交、推送并调整仓库可见性；推送后确认远端 CI 结果。
- 在 GitHub 设置中启用私密漏洞报告，并确认 [SECURITY.md](../SECURITY.md) 中的联系路径可用。

```powershell
cargo fmt --all --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build -p omnitrigger-app --locked
.\target\debug\omnitrigger.exe --smoke-test
git diff --check
```

## 分发 Windows 程序前

这是发布程序时需要额外完成的工作，不能仅将调试目录内的 EXE 上传作为正式发行包。

1. 确认版本、目标架构及更新说明，使用 `cargo build -p omnitrigger-app --release --locked` 构建。默认输出为 `target\release\omnitrigger.exe`，显式指定 `--target` 或 `CARGO_TARGET_DIR` 时路径相应改变。
2. 根据实际发行目标与启用的功能，为锁定依赖生成完整的第三方许可、版权和适用通知；与项目 LICENSE 一起放入发行包。详见 [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md)。
3. 在干净的 Windows 环境检查启动、图标、中文输入、配置保存、前缀回退、录制与触发、暂停/恢复、重建监听和退出；检查未安装开发工具时所需的运行库。
4. 包内不含本机配置、PDB、真实日志或临时测试文件。若程序未经签名，应在发行说明中如实注明。
5. 计算发行包的 SHA-256 校验值，并在发行说明中写清支持系统、架构、已知限制和手工验证结果。由维护者确认后发布。
