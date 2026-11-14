# 从源码构建

本页说明从源码构建、运行和检查 OmniTrigger。只想试用时，下载 Windows 预览版即可，见[快速开始](GETTING_STARTED.md)。

## 环境要求

需要 Windows、Rust（MSVC 工具链）及 Visual Studio C++ Build Tools（含“使用 C++ 的桌面开发”和 Windows SDK）。仓库通过 `rust-toolchain.toml` 固定 Rust 1.90.0、rustfmt 与 Clippy；脚本需要 PowerShell 7。

```powershell
git clone https://github.com/ceastld/omnitrigger.git
cd omnitrigger
.\start.ps1                                      # 构建并打开本机测试配置档
cargo run -p omnitrigger-app --locked -- --test-workspace
```

用 `target\debug\omnitrigger.exe --test-workspace` 可再次打开同一测试配置档。直接运行无参数 EXE 会打开日常配置。

上述路径为默认 Cargo 输出目录；`start.ps1` 和检查脚本共用 `scripts/build-app.ps1`，从本次 Cargo 构建回执定位 EXE，支持自定义 `CARGO_TARGET_DIR` 与 Windows 构建目标目录。非 Windows 环境可运行 `cargo test -p omnitrigger-core --locked` 验证核心；桌面程序仍仅面向 Windows。

## 构建和检查

```powershell
.\scripts\check.ps1                               # 与 Windows CI 共用，包含示例配置检查
cargo build -p omnitrigger-app --locked
cargo test --workspace --locked
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

真实 Hook 检查需要交互式 Windows 桌面。请在空闲的 Windows 会话中运行；同一会话已有新版 OmniTrigger 监听时，检查会报告占用并退出。发行 CI 验证实例之间的监听占用和释放，实际按键、输入焦点与权限差异仍需实机验证。

```powershell
.\target\debug\omnitrigger.exe --smoke-test
cargo run -p omnitrigger-windows --locked --example hook_smoke
```

## 隔离试用与配置检查

通过 `--config <文件路径>` 使用独立配置，例如 `.\target\debug\omnitrigger.exe --config .\.local\demo\bindings.json`；后续编辑会保存到该文件，适合隔离试用。

需要反复使用的测试数据用测试配置档：数据保存在数据根的 `profiles\` 下，每次启动读写同一份，默认不安装 Hook，窗口标题与侧栏标明“测试数据”。

```powershell
.\target\debug\omnitrigger.exe --profile-create ui-browse --seed fixture:tests\fixtures\profiles\ui-browse --name 浏览视图
.\target\debug\omnitrigger.exe --profile ui-browse           # 只打开界面
.\target\debug\omnitrigger.exe --profile ui-browse --listen  # 需要验证真实输入时
.\target\debug\omnitrigger.exe --profile-reset ui-browse     # 恢复种子数据
.\target\debug\omnitrigger.exe --profile-list
```

`--data-root <目录>` 或环境变量 `OMNITRIGGER_HOME` 可改用另一个数据根。从打包应用（例如桌面 Agent 客户端）的终端启动时，程序会检测到系统对 `%LOCALAPPDATA%` 的重定向，并经系统外壳重启自身，确保读写真实数据目录。详见[配置档与数据目录设计](PROFILES.md)。

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

## 显式事件回放

开发测试可读取独立 JSON 见证，经配置解码后交给纯核心，逐步核对 `triggered`、`replay` 和可选的 `consume`。它不安装 Hook、不注入输入、不运行绑定动作，也不保存配置：

```powershell
$env:OMNITRIGGER_CAPTURE_REPLAY = 'D:\Test\replay.json'
cargo test --locked -p omnitrigger-app captured_matching_replays_external_source_witnesses -- --ignored --test-threads=1 --nocapture
```

样本含 `config` 和 `cases`。`config` 使用当前绑定格式，所有绑定须为 `enabled:false`。每组独立见证设置 `reset:true`，重新建立引擎和时钟；缺省沿用上组状态以兼容已有连续样本，时间只会单调前进。`process`、`role` 是初始前台上下文，可用 `pointer_process`、`pointer_role` 指定独立指针上下文。完整示例：

```json
{
  "config": {
    "version": 17,
    "bindings": [{
      "id": 1, "held": [{"kind":"x1"}],
      "activation": {"kind":"wheel_up"}, "wheel": "packet",
      "pointer_scope": {"allow":["reader"]},
      "message": "回放样本", "enabled": false,
      "action": {"kind":"notification"}
    }]
  },
  "cases": [{
    "reset": true, "process": "editor", "role": "application",
    "pointer_process": "reader", "pointer_role": "application",
    "steps": [
      {"event":{"kind":"down","button":{"kind":"x1"}}, "time_ms":0, "triggered":[], "consume":true},
      {"event":{"kind":"wheel","axis":"vertical","delta":1}, "time_ms":1, "triggered":[1], "consume":true},
      {"event":{"kind":"up","button":{"kind":"x1"}}, "time_ms":2, "triggered":[], "consume":true}
    ]
  }]
}
```

键盘按钮为 `{"kind":"key","value":75}`；兼容原来的步骤写法 `{"key":75,"down":true,"triggered":[]}`。输入步骤可带 `injected:true`，核心忽略此输入且不推进时间。滚轮 `axis` 为 `vertical` 或 `horizontal`，保留原始有符号 `delta`。`replay` 数组使用相同的输入格式，省略时预期没有补发。

其他 `event.kind` 为 `advance`（须带 `time_ms`）、`foreground`、`pointer`、`cancel`。两个上下文事件使用 `process`（字符串或 `null`）与可选 `role`；角色为 `unknown`、`application`、`desktop` 或 `taskbar`。上下文变更或取消前需单独推进时间，不在该步骤附带 `time_ms`，其收尾决策同样需要核对。文件最多 32 MiB、100000 组，每组 1–32 步。

回放通过只证明所列事件与上下文下的配置行为；装载了多少规则和专门验证了多少规则须分别记录。物理输入、平台采样、目标程序效果及与外部行为的等价性仍需独立验证。

## 界面设计预览

`design/` 用 React 组件预览触发器页的列表、键盘和鼠标视图，数据由应用 `--export-trigger-preview` 导出，改界面时先在这里迭代再移植到 GPUI。用法见 [design/README.md](../design/README.md)。

## 主页与文档站

主页与文档站位于 `site/`，使用 VitePress 构建，需要 Node.js 20 或更高版本。文档页由 `site/scripts/sync-docs.mjs` 从仓库 Markdown 同步生成，仓库文档仍是唯一来源；`docs/` 下新增文档需要在该脚本中登记侧栏位置。

```powershell
cd site
npm ci
npm run dev       # 本地预览，修改仓库文档后自动同步
npm run build     # 生成 site/.vitepress/dist，并检查文档链接
```

推送到 `main` 后，`Pages` 工作流构建并发布到 GitHub Pages。代码边界、验证要求与提交规范见[贡献指南](../CONTRIBUTING.md)。
