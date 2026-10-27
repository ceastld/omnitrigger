# 开源与发布检查

目前只分发 Windows x64 开发预览版。日常 CI 与 `scripts/check.ps1` 共用格式、测试、Clippy、构建和示例配置检查，Linux CI 只验证核心，不生成发行包。独立的 Windows Release 工作流在版本标签推送后构建、打包并发布预览版；手动运行只生成 Actions 产物，不公开 Release。三个 crate 仍设 `publish = false`，没有发布到 crates.io。

## 公开源码前

- 核对 README 的功能、支持平台与命令，保留早期开发状态和实际输入限制。
- 检查待公开的文件及 Git 历史，不包含凭据、个人配置、真实输入记录、本机日志或构建产物；不要用重写历史代替必要的凭据撤销。
- 临时试用文件放入已忽略的 `.local/` 或 `.tmp/`；确认忽略规则已提交到 `.gitignore`，不能只依赖个人的 `.git/info/exclude`。
- 确认新增代码和素材具备授权，保留 [LICENSE](../LICENSE) 与[第三方说明](../THIRD_PARTY_NOTICES.md)。
- 执行下列检查，查看 `git diff --check` 与最终差异；实际 Hook 行为在交互式 Windows 桌面中验证。
- 由维护者确认公开范围后提交、推送并调整仓库可见性；推送后确认远端 CI 结果。
- 在 GitHub 设置中启用私密漏洞报告，并确认 [SECURITY.md](../SECURITY.md) 中的联系路径可用。

```powershell
.\scripts\check.ps1 -SmokeTest
git diff --check
```

脚本需要 PowerShell 7 和交互式 Windows 桌面；CI 不指定 `-SmokeTest`。示例检查只读取配置，不运行快捷键、程序或系统调节。固定工具链见 `rust-toolchain.toml`；脚本共用 `scripts/build-app.ps1`，从本次构建回执定位 EXE，避免使用自定义目标目录之外的旧产物。

源码公开不要求可选插件下载可用；若插件发布包仍需仓库访问权限，README 和动作库必须如实说明，不能将它写成所有用户均可匿名安装的内置能力。

## 分发 Windows 程序前

发行包由 [`scripts/package-windows.ps1`](../scripts/package-windows.ps1) 生成，不直接分发调试 EXE：

```powershell
.\scripts\package-windows.ps1 -Target x86_64-pc-windows-msvc
```

脚本使用锁定依赖和 Release 配置，从本次 Cargo 回执定位 EXE，生成 `dist\OmniTrigger-<版本>-windows-x86_64.zip` 和 `dist\SHA256SUMS.txt`。包内包含项目许可证、运行说明、发布说明及按实际目标收集的第三方许可 / 版权 / NOTICE 和机器可读清单；缺少所需许可时打包失败。许可文件来源与补充规则见 [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md)。

发布前核对：

1. 工作区版本与 `docs/releases/v<版本>.md` 一致，运行说明明确 Windows 架构、VC++ 运行库要求、未签名状态及真实已知限制。
2. 在独立目录解压 ZIP，检查启动、图标、中文输入、配置保存、前缀回退、录制与触发、暂停 / 恢复、重建监听和退出。尽可能在未安装开发工具的 Windows 环境验证；未覆盖的环境必须在发布说明中如实注明。
3. 包内不含本机配置、PDB、真实日志或临时测试文件。可选下载的外部插件不合并进发行包。
4. 核对 SHA-256、许可清单、最终 Git 差异，以及发布说明中手工与自动验证的实际范围。

## 通过 GitHub Actions 发布

[`Windows Release`](../.github/workflows/release.yml) 工作流固定生成 `x86_64-pc-windows-msvc` 发行包：

- 普通分支推送运行日常 CI，不自动创建 Release。
- 手动运行 Windows Release 只构建并保留 Actions artifact，可先下载检查。
- 推送 `v<版本>` 标签后，工作流核对标签与 Cargo 版本一致，运行检查、构建和许可收集。通过后，由单独的写权限作业核对标签提交和 ZIP 哈希，先上传草稿资产，再公开为预发布版；不会标记为稳定版或 Latest。
- 同一标签已有 Release（含草稿）时停止，不覆盖原资产。失败后先检查原因与草稿，修复代码通常使用新版本和新标签；不要静默移动已公开标签。
- Actions 不改变仓库可见性，不创建其他平台安装包，不修改账单或自动启用付费 runner。

维护者确认版本和范围后执行，例如首版：

```powershell
git push origin main
git tag -a v0.1.0 -m "release: 发布 Windows 预览版 0.1.0"
git push origin v0.1.0
```

后续版本先更新 `Cargo.toml` 的工作区版本、同步 `Cargo.lock` 中本项目包版本及对应发布说明，再创建匹配标签。发布后核对 Actions 两个作业、公开 Release、资产文件名、大小和 SHA-256。
