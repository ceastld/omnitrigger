# 开源与发布检查

目前只分发 Windows x64 开发预览版。日常 CI 与 `scripts/check.ps1` 共用格式、测试、Clippy、构建和示例配置检查，Linux CI 只验证核心，不生成发行包。独立的 Windows Release 工作流在版本标签推送后构建、打包并发布预览版；手动运行只生成 Actions 产物，不公开 Release。三个 crate 仍设 `publish = false`，没有发布到 crates.io。

## 公开源码前

- 核对 README 的功能、支持平台与命令，保留早期开发状态和实际输入限制；主页与文档站的描述保持一致，`site` 构建通过。
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

发行脚本需要 Inno Setup 6，使用 [`scripts/package-windows.ps1`](../scripts/package-windows.ps1) 从同一份 Release 文件生成安装器和便携版。

将鼠标悬停在日常实例的托盘图标上即可核对运行版本。日常档和测试档的“设置”页首也会显示当前窗口的产品版本。CI 安装包会在版本号后附上构建提交的前 12 位，便于区分同版本的预览包。

运行 `omnitrigger.exe --build-version` 可以在后台核对该程序文件的产品版本。安装器和程序文件的“属性 → 详细信息 → 产品版本”也显示这一值。`--version` 继续输出应用版本号。

日常工作区的监听就绪后会向 Windows 登记原配置文件。普通覆盖安装若通过 Restart Manager 关闭这个实例，Windows 会按原路径从托盘恢复监听。暂停监听会撤销登记，恢复监听后重新登记。测试档独立运行。Windows 要求进程已运行至少 60 秒才会自动重启。[Windows 应用重启说明](https://learn.microsoft.com/windows/win32/api/winbase/nf-winbase-registerapplicationrestart)

首次安装、从尚未支持登记的旧版升级，或需要直接启动安装器所选的日常档时，可传入 `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /NORESTARTAPPLICATIONS /STARTINTRAY=1`。这会由安装器启动一次托盘入口。开始菜单或托盘图标可以打开同一窗口。已安装的程序也支持 `omnitrigger.exe --tray`，重复运行会沿用现有日常实例。后台启动会跳过外部副本提示，下一次正常启动仍可处理该副本。

要在以后每次登录 Windows 时启动日常监听，可在安装器中勾选“登录 Windows 时在托盘启动 OmniTrigger”。安装器会把带 `--tray` 参数的快捷方式放进当前用户的启动文件夹。静默安装可用 `/MERGETASKS=logintray` 开启，或用 `/MERGETASKS=!logintray` 关闭。省略参数会沿用上一次安装的选择。全新安装默认关闭，卸载会移除该启动项。

确认卸载后，卸载器会请当前安装目录的日常实例和测试窗口退出，然后移除程序。其他目录的便携版可以继续使用。若其他程序仍占用 `omnitrigger.exe` 并阻止删除，卸载器会停止，解除占用后可重新卸载。日常与测试配置保留在各自的数据目录中。

```powershell
.\scripts\package-windows.ps1 -Target x86_64-pc-windows-msvc
```

脚本使用锁定依赖，从本次 Cargo 回执定位 EXE，以该 EXE 只读验证公开 JSON 示例。产物位于 `dist`，包括 `OmniTrigger-<版本>-windows-x86_64-setup.exe`、同名前缀的 ZIP 和列出两项摘要的 `SHA256SUMS.txt`。两种包均包含项目许可证、运行说明、发布说明、目标依赖许可及机器可读清单。缺少许可正文时打包失败。许可文件来源见 [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md)。

发行 CI 静默安装程序并核对两个开始菜单入口。它也会启用登录启动项，通过该快捷方式核对托盘监听就绪，并在覆盖安装、取消勾选和卸载后检查快捷方式是否与选择一致。日常和测试配置各运行一次剪贴板动作包，分别保存同一条合成文本。首次安装后启动日常窗口，确认监听就绪、关窗收进托盘，再运行安装器覆盖安装，核对旧进程退出和程序摘要。覆盖安装及卸载后检查配置文件的哈希。重装后再次执行剪贴板动作，两份历史都应识别已有记录。

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

后续版本先更新 `Cargo.toml` 的工作区版本、同步 `Cargo.lock` 中本项目包版本及对应发布说明，再创建匹配标签。发布后核对构建、发布、更新服务通知作业，以及公开 Release、资产文件名、大小和 SHA-256。

## 应用内更新契约

设置页更新器优先读取 `https://update.alinko.cc/omnitrigger/version.json`；网络或清单校验失败时回退官方 GitHub 最近 100 条公开 Release（包含预发布版），按 SemVer 选择高于当前版本的最高版本。启动就绪后检查一次，此后每 5 小时检查；下载和安装始终需用户操作。当前版本来自 Cargo 构建版本；不得仅更改发布标签而不更新工作区版本。

发行资产继续使用 `OmniTrigger-<版本>-windows-x86_64.zip` 和 `SHA256SUMS.txt`，清单行格式为 `<64 位 SHA-256>  <ZIP 文件名>`；ZIP 必须具有 GitHub 资产 SHA-256 摘要。ZIP 根目录保留 `omnitrigger.exe`、`LICENSE`、`THIRD_PARTY_NOTICES.md`、`README.txt`、`RELEASE_NOTES.md` 和 `licenses/`。更新器拒绝其他顶层文件、路径穿越、重复文件、链接及超限解压内容；以后改变发行布局时需同步更新器。

版本清单由 [ceastld/app-updates](https://github.com/ceastld/app-updates) 的 `public/omnitrigger/version.json` 维护，通过同一 Workers 和 `update.alinko.cc` 域名分发。字段为 `product=omnitrigger`、`release`（Cargo SemVer）、`version`、`publishedAt`、`downloadUrl`、`sha256SumsUrl`、`sha256`、`size`、`notes`。三段数字的展示版本追加 `.0`，带预发布后缀时保持原 SemVer；客户端始终按 `release` 比较。

Windows Release 在成功公开并校验发行资产后，显式调用[通知工作流](../.github/workflows/notify-update-api.yml)，向 `app-updates` 发送 `release-published` 事件。手动公开 Release 也由 `release: published` 事件通知；可手动运行通知工作流并传入已有标签补发。接收端校验产品和仓库，拒绝版本倒退和同版本 ZIP 摘要变化，随后同步、部署并读回 JSON 正文及 JSON/CORS 响应头。不创建测试 Release，不使用服务端轮询。

跨仓库通知需在本仓库 Actions Secrets 配置 `APP_UPDATES_DISPATCH_TOKEN`，仅授予 `ceastld/app-updates` 的 Contents: write 权限。此密钥不进入源码；普通分支推送和应用内检查均不需要它。首次接入可用既有公开版本同步、部署和验证服务，无需创建新标签。

网络与解包由后台线程启动的隐藏 PowerShell 下载器完成。普通关窗会将日常窗口收至托盘。替换器完成预检后，主界面才为本次更新写入退出授权，同时记录窗口是否已收起。替换器收到授权并等到旧进程退出，才备份和覆盖发行文件。

新版沿用原配置路径。若重启前窗口已收起，更新完成或回滚后会继续在托盘监听，可从托盘图标或开始菜单打开。若窗口仍然打开，重启后会显示设置页。新版创建工作区且输入后端就绪后，后台写完版本回执再将其发布。替换器在 30 秒的启动期限内等待读取回执，遇到短暂的文件占用会重试。确认进程仍在运行且版本与发行包一致后才删除备份。启动失败时会尝试回滚，重新打开设置可以查看原因。短暂映像共享锁会有界重试，不结束其他实例。此恢复流程处理进程内检测到的失败，不提供断电或操作系统崩溃后的事务恢复。

新增版本的发布前人工验证应包含：统一 API 读回、备用源回退、后台下载进度、真实发行包完整校验、自动重启、配置保留、文件被占用、不可写目录及启动失败后的恢复。无网络测试覆盖清单身份、摘要、定时检查、回退、ZIP 边界、退出授权、中文带空格路径、成功替换和失败回滚。SHA-256 由 GitHub 资产摘要同步到 Cloudflare，同时核对原始 SHA256SUMS；这不替代独立发行签名。
